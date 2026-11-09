# Place and route a gate-level sky130_fd_sc_hd netlist with OpenROAD.
#
# Inputs (environment):
#   PLATFORM_DIR   ORFS sky130hd platform directory (LEF, liberty, PDN, tracks)
#   NETLIST, TOP   structural Verilog and its top module
#   OUT_DIR        where DEF/ODB and reports are written
#   MAX_LAYER      highest metal layer for signal routing (met1..met5, default met5)
#   CLOCK_PORT     clock input; empty for purely combinational designs
#   CLOCK_PERIOD   clock period in ns (default 10)
#   DIE_SIZE       square die edge in um; otherwise sized by CORE_UTIL (percent, default 30)
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

# --- Floorplan, power grid, pins -----------------------------------------------
if {[env_or DIE_SIZE ""] ne ""} {
    set d $::env(DIE_SIZE)
    initialize_floorplan -die_area "0 0 $d $d" -core_area "5 5 [expr {$d - 5}] [expr {$d - 5}]" -site unithd
} else {
    initialize_floorplan -utilization [env_or CORE_UTIL 30] -aspect_ratio 1 -core_space 5 -site unithd
}
source $platform/make_tracks.tcl

set ::env(TAP_CELL_NAME) sky130_fd_sc_hd__tapvpwrvgnd_1
source $platform/tapcell.tcl
# Tap cells are new instances: connect their supply pins too.
global_connect
source $platform/pdn.tcl
pdngen

place_pins -hor_layers met3 -ver_layers met2
source $platform/setRC.tcl

# --- Placement and clock tree --------------------------------------------------
global_placement -density 0.6
detailed_placement

if {$clk_port ne ""} {
    clock_tree_synthesis -root_buf sky130_fd_sc_hd__clkbuf_16 -buf_list sky130_fd_sc_hd__clkbuf_4
    set_propagated_clock [all_clocks]
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
if {$clk_port ne ""} {
    report_checks -path_delay max -format full_clock_expanded > $out/timing.rpt
    report_worst_slack
}
