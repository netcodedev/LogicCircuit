// Toolchain smoke test (Phase 0): a registered half adder built directly from
// sky130_fd_sc_hd cells. It exercises combinational cells, flip-flops and a
// clock through floorplan, placement and routing.
module smoke (
    input  wire CLK,
    input  wire a,
    input  wire b,
    output wire sum,
    output wire carry
);
    wire s, c;

    sky130_fd_sc_hd__xor2_1  u_xor (.A(a), .B(b), .X(s));
    sky130_fd_sc_hd__and2_1  u_and (.A(a), .B(b), .X(c));
    sky130_fd_sc_hd__dfxtp_1 u_sum   (.CLK(CLK), .D(s), .Q(sum));
    sky130_fd_sc_hd__dfxtp_1 u_carry (.CLK(CLK), .D(c), .Q(carry));
endmodule
