//! End-to-end formatter checks against representative VHDL files.
//!
//! These run the *public* `format` API the way an editor would, so a
//! regression in any of the underlying passes will trip a test here
//! even if the per-pass unit tests still happen to pass.

const XILINX_TEMPLATE_UNLABELED: &str = "\
library IEEE;
use IEEE.STD_LOGIC_1164.ALL;

entity test is
  port (
    test_in  : in  std_logic;
    clk      : in  std_logic;
    test_out : out std_logic
  );
end test;

architecture Behavioral of test is

begin

  process(clk)
  begin
    if rising_edge(clk) then
    end if;
  end process;

end Behavioral;
";

#[test]
fn xilinx_template_unlabeled_process_is_labeled() {
    let out = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED).unwrap();
    assert!(
        out.contains("u_process_1 : process(clk)"),
        "head label missing:\n{out}"
    );
    assert!(
        out.contains("end process u_process_1;"),
        "tail label missing:\n{out}"
    );
    // The rest of the file must be byte-identical except for our
    // insertions. We check by removing both labels and comparing
    // against the original.
    let stripped = out
        .replace("u_process_1 : ", "")
        .replace(" u_process_1", "");
    assert_eq!(stripped, XILINX_TEMPLATE_UNLABELED);
}

#[test]
fn idempotent_on_already_formatted_input() {
    let once = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED).unwrap();
    let twice = hdlfmtr::format(&once).unwrap();
    assert_eq!(once, twice, "format is not idempotent");
}
