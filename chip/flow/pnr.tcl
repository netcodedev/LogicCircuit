# Place and route a gate-level sky130_fd_sc_hd netlist with OpenROAD.
#
# Inputs (environment):
#   PLATFORM_DIR   ORFS sky130hd platform directory (LEF, liberty, PDN, tracks)
#   NETLIST, TOP   structural Verilog and its top module
#   OUT_DIR        where DEF/ODB and reports are written
#   MAX_LAYER      highest metal layer for routing (met2..met5, default met5); the
#                  power grid and the pins are kept at or below it as well
#   CLOCK_PORT     clock input; empty for purely combinational designs
#   CLOCK_PERIOD   clock period in ns (default 10)
#   DIE_SIZE       die size in um, "W" (square) or "WxH"; otherwise sized by CORE_UTIL (percent, default 30)
#   NO_CTS         1 = skip clock tree synthesis (designs without flip-flop cells:
#                  with our own latches CTS follows the clock into the latch gates
#                  and crashes); the clock is then routed like any signal
#   OPTIMIZE       1 = let OpenROAD insert buffers and resize cells (repair_design,
#                  repair_timing); the netlist then no longer matches ours 1:1
#   MEM_DELAY      access time in ns of the memories outside the chip (default 0 =
#                  ideal: every input arrives with the clock edge). The instruction
#                  memory answers PC with Instruction/InstructionNext, the data
#                  memory MemAddr with MemDataIn, each MEM_DELAY after its address
#                  is stable; see apply_mem_delay
#
# Besides DEF/ODB/Verilog it writes metrics.txt ("key value" lines): die size,
# cell area, instances and transistors per cell master. Wire length per
# layer, via count and slack are read from the log by the Rust CLI.
#
# The netlist is used as is: no synthesis, so every cell we export ends up on
# the chip.

proc env_or {name default} {
    if {[info exists ::env($name)] && $::env($name) ne ""} { return $::env($name) }
    return $default
}

set platform  $::env(PLATFORM_DIR)
set top       $::env(TOP)
set out       $::env(OUT_DIR)
set max_layer [env_or MAX_LAYER met5]
set clk_port  [env_or CLOCK_PORT ""]
set optimize  [expr {[env_or OPTIMIZE 0] eq "1"}]
set layers    {met1 met2 met3 met4 met5}
if {[lsearch $layers $max_layer] < 1} {
    error "MAX_LAYER must be one of met2..met5, got '$max_layer'"
}
file mkdir $out

# --- Technology and design ---------------------------------------------------
read_lef $platform/lef/sky130_fd_sc_hd.tlef
read_lef $platform/lef/sky130_fd_sc_hd_merged.lef
read_liberty $platform/lib/sky130_fd_sc_hd__tt_025C_1v80.lib
read_verilog $::env(NETLIST)
link_design $top

source $platform/global_connect.tcl
global_connect

if {$clk_port ne ""} {
    create_clock -name clk -period [env_or CLOCK_PERIOD 10] [get_ports $clk_port]
    # Treat chip pins as launched/captured by the same clock, so that
    # input -> flip-flop and flip-flop -> output paths are timed too.
    set data_inputs [lsearch -all -inline -not -exact [all_inputs] [get_ports $clk_port]]
    set_input_delay 0 -clock clk $data_inputs
    set_output_delay 0 -clock clk [all_outputs]
}

# Memories outside the chip, modelled as input delays: a read port answers
# MEM_DELAY ns after the latest bit of its address bus arrives at the chip
# pins. With output delay 0, a port arrives at (period - slack). PC must be
# timed before Instruction (the data address may depend on the instruction),
# and MemAddr must not depend on MemDataIn (true for CPU_16B). The data ports
# also get a buf_4 as driver. Called once before placement (rough, for
# --optimize) and again before the final report.
proc apply_mem_delay {} {
    set t [env_or MEM_DELAY 0]
    if {$::clk_port eq "" || $t == 0} { return }
    set period [env_or CLOCK_PERIOD 10]
    # Separate instruction and data ports, or one memory bus (CPU_16B_SOC,
    # SOC_16B: Addr -> DataIn, Wait).
    foreach {addr data} {PC {Instruction InstructionNext} MemAddr {MemDataIn} Addr {DataIn Wait}} {
        set addr_ports [get_ports -quiet "${addr}\[*\]"]
        if {[llength $addr_ports] == 0} { continue }
        set arrival 0.0
        foreach p $addr_ports {
            set a [expr {$period - [get_property $p slack_max]}]
            if {$a > $arrival} { set arrival $a }
        }
        set delay [expr {$arrival + $t}]
        foreach d $data {
            set ports [get_ports -quiet "${d}\[*\]"]
            if {[llength $ports] > 0} {
                set_input_delay $delay -clock clk $ports
                # A memory drives its outputs through ordinary buffers, not
                # the ideal driver STA assumes for ports; with fanout to
                # hundreds of gates (register addresses, control) that matters.
                set_driving_cell -lib_cell sky130_fd_sc_hd__buf_4 $ports
                puts [format "mem_delay %s -> %s: address at %.2f ns, data at %.2f ns" $addr $d $arrival $delay]
            }
        }
    }
}
apply_mem_delay

# --- Floorplan, power grid, pins -----------------------------------------------
if {[env_or DIE_SIZE ""] ne ""} {
    set wh [split [string tolower $::env(DIE_SIZE)] x]
    set w [lindex $wh 0]
    set h [expr {[llength $wh] > 1 ? [lindex $wh 1] : $w}]
    initialize_floorplan -die_area "0 0 $w $h" -core_area "5 5 [expr {$w - 5}] [expr {$h - 5}]" -site unithd
} else {
    # Size the core by utilisation, but not below MIN_CORE um: the power grid
    # needs room for its straps (pitch ~27 um), tiny designs would not fit.
    set min_core 60
    set block [ord::get_db_block]
    set dbu [$block getDbUnitsPerMicron]
    set cell_area 0.0
    foreach inst [$block getInsts] {
        set m [$inst getMaster]
        set cell_area [expr {$cell_area + double([$m getWidth]) * [$m getHeight] / $dbu / $dbu}]
    }
    set util [env_or CORE_UTIL 30]
    if {sqrt($cell_area / ($util / 100.0)) < $min_core} {
        set d [expr {$min_core + 10}]
        initialize_floorplan -die_area "0 0 $d $d" -core_area "5 5 [expr {$d - 5}] [expr {$d - 5}]" -site unithd
    } else {
        initialize_floorplan -utilization $util -aspect_ratio 1 -core_space 5 -site unithd
    }
}
source $platform/make_tracks.tcl

set ::env(TAP_CELL_NAME) sky130_fd_sc_hd__tapvpwrvgnd_1
source $platform/tapcell.tcl
# Tap cells are new instances: connect their supply pins too.
global_connect
# Power grid: the platform grid uses met4/met5 stripes. With fewer layers,
# build a reduced grid: met1 rails plus vertical stripes on the highest
# vertical layer available (met4 or met2; sky130 alternates H/V from met1).
if {$max_layer eq "met5"} {
    source $platform/pdn.tcl
} else {
    set stripe [expr {$max_layer eq "met2" || $max_layer eq "met3" ? "met2" : "met4"}]
    set_voltage_domain -name {CORE} -power {VDD} -ground {VSS}
    define_pdn_grid -name {grid} -voltage_domains {CORE} -pins $stripe
    add_pdn_stripe -grid {grid} -layer {met1} -width {0.48} -pitch {5.44} -offset {0} -followpins
    add_pdn_stripe -grid {grid} -layer $stripe -width {1.2} -pitch {27.14} -offset {13.57}
    add_pdn_connect -grid {grid} -layers [list met1 $stripe]
}
pdngen

# Pins on the highest horizontal / vertical layers that may be used.
set pin_h [expr {$max_layer eq "met2" ? "met1" : "met3"}]
place_pins -hor_layers $pin_h -ver_layers met2
source $platform/setRC.tcl

# --- Placement and clock tree --------------------------------------------------
global_placement -density 0.6
if {$optimize} {
    estimate_parasitics -placement
    repair_design
}
detailed_placement

if {$clk_port ne "" && [env_or NO_CTS 0] ne "1"} {
    clock_tree_synthesis -root_buf sky130_fd_sc_hd__clkbuf_16 -buf_list sky130_fd_sc_hd__clkbuf_4
    set_propagated_clock [all_clocks]
    if {$optimize} {
        estimate_parasitics -placement
        repair_timing -setup
    }
    detailed_placement
}

filler_placement {sky130_fd_sc_hd__fill_1 sky130_fd_sc_hd__fill_2 sky130_fd_sc_hd__fill_4 sky130_fd_sc_hd__fill_8}

# --- Routing -------------------------------------------------------------------
set ::env(MIN_ROUTING_LAYER) met1
set ::env(MIN_CLK_ROUTING_LAYER) met1
set ::env(MAX_ROUTING_LAYER) $max_layer
source $platform/fastroute.tcl
global_route
# verbose 1 prints the final per-layer wire length and via counts.
detailed_route -output_drc $out/drc.rpt -verbose 1

# --- Results -------------------------------------------------------------------
write_def $out/$top.def
write_db $out/$top.odb
write_verilog $out/$top.routed.v

report_design_area
apply_mem_delay
if {$clk_port ne ""} {
    report_checks -path_delay max -format full_clock_expanded > $out/timing.rpt
    report_worst_slack
    # The worst path to every endpoint, one line each ("startpoint endpoint
    # slack_ns"), worst first: shows how long the paths of each part are, not
    # just the single worst one. Optional: a failure here keeps the run.
    if {[catch {
        set f [open $out/endpoints.txt w]
        foreach pe [find_timing_paths -path_delay max -group_path_count 100000 -endpoint_path_count 1 -sort_by_slack] {
            set sp [get_full_name [get_property $pe startpoint]]
            set ep [get_full_name [get_property $pe endpoint]]
            puts $f "$sp $ep [format %.3f [get_property $pe slack]]"
        }
        close $f
    } err]} {
        puts "endpoints.txt: $err"
    }
}

# --- Metrics -------------------------------------------------------------------
# Transistors per cell master, counted from the platform's SPICE netlist.
proc cdl_transistors {path} {
    set counts [dict create]
    set fh [open $path]
    set cell ""
    while {[gets $fh line] >= 0} {
        set word [string toupper [lindex [split [string trim $line]] 0]]
        if {$word eq ".SUBCKT"} {
            set cell [lindex [split [string trim $line]] 1]
            dict set counts $cell 0
        } elseif {$word eq ".ENDS"} {
            set cell ""
        } elseif {$cell ne "" && [string match {M*} [string toupper [string trim $line]]]} {
            dict incr counts $cell
        }
    }
    close $fh
    return $counts
}
set transistors_of [cdl_transistors $platform/cdl/sky130hd.cdl]

set block [ord::get_db_block]
set dbu [$block getDbUnitsPerMicron]
set die [$block getDieArea]
set masters [dict create]
set cell_area 0.0
set logic_area 0.0
set transistors 0
foreach inst [$block getInsts] {
    set m [$inst getMaster]
    set name [$m getName]
    dict incr masters $name
    set a [expr {double([$m getWidth]) * [$m getHeight] / $dbu / $dbu}]
    set cell_area [expr {$cell_area + $a}]
    # Logic area: without filler and tap cells, which only fill rows.
    if {![string match *__fill_* $name] && ![string match *__tapvpwrvgnd_* $name]} {
        set logic_area [expr {$logic_area + $a}]
    }
    if {[dict exists $transistors_of $name]} {
        incr transistors [dict get $transistors_of $name]
    }
}
set fh [open $out/metrics.txt w]
puts $fh "die_width_um [expr {double([$die dx]) / $dbu}]"
puts $fh "die_height_um [expr {double([$die dy]) / $dbu}]"
puts $fh "cell_area_um2 [format %.1f $cell_area]"
puts $fh "logic_area_um2 [format %.1f $logic_area]"
puts $fh "transistors $transistors"
puts $fh "max_layer $max_layer"
puts $fh "clock_period_ns [env_or CLOCK_PERIOD 10]"
puts $fh "optimize $optimize"
dict for {name n} $masters {
    puts $fh "cell $name $n"
}
close $fh
