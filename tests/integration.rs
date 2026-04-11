//! End-to-end formatter checks against representative VHDL files.
//!
//! These run the *public* `format` API the way an editor would, so a
//! regression in any of the underlying passes will trip a test here
//! even if the per-pass unit tests still happen to pass.

use hdlfmtr::{Config, IndentStyle, KeywordCase};

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

/// Config that disables every pass except label insertion. Used to
/// keep label-pass tests focused.
fn labels_only() -> Config {
    Config {
        indent_style: IndentStyle::Preserve,
        indent_width: 4,
        keyword_case: KeywordCase::Preserve,
    }
}

#[test]
fn xilinx_template_unlabeled_process_is_labeled() {
    let out = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &labels_only()).unwrap();
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
    let once = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &labels_only()).unwrap();
    let twice = hdlfmtr::format(&once, &labels_only()).unwrap();
    assert_eq!(once, twice, "format is not idempotent");
}

#[test]
fn default_config_full_pipeline_is_idempotent() {
    let cfg = Config::default();
    let once = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &cfg).unwrap();
    let twice = hdlfmtr::format(&once, &cfg).unwrap();
    assert_eq!(once, twice, "default-config format is not idempotent");
}

#[test]
fn default_config_emits_tabs_and_lowercase_keywords() {
    let cfg = Config::default();
    let out = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &cfg).unwrap();
    // Tabs should appear as indent characters somewhere.
    assert!(out.contains('\t'), "expected tabs in output:\n{out}");
    // The original "Behavioral" architecture name keeps its case
    // (it's an identifier), but reserved words like `IN` / `OUT` /
    // `PORT` get folded to lowercase.
    assert!(out.contains(" in "), "expected lowercased `in`:\n{out}");
    assert!(out.contains(" out "), "expected lowercased `out`:\n{out}");
    // Trailing newline guaranteed.
    assert!(out.ends_with('\n'));
}

#[test]
fn spaces_indent_with_width_2() {
    let cfg = Config {
        indent_style: IndentStyle::Spaces,
        indent_width: 2,
        keyword_case: KeywordCase::Preserve,
    };
    let out = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &cfg).unwrap();
    // No tabs at all.
    assert!(!out.contains('\t'), "expected no tabs:\n{out}");
    // Two-space indent should appear at depth 1 lines.
    assert!(
        out.contains("\n  port ("),
        "expected 2-space indent before `port (`:\n{out}"
    );
}

#[test]
fn upper_keyword_case() {
    let cfg = Config {
        indent_style: IndentStyle::Preserve,
        indent_width: 4,
        keyword_case: KeywordCase::Upper,
    };
    let out = hdlfmtr::format(XILINX_TEMPLATE_UNLABELED, &cfg).unwrap();
    assert!(out.contains("ENTITY"), "expected uppercased keyword:\n{out}");
    assert!(out.contains("PROCESS"));
    assert!(out.contains("BEGIN"));
}
