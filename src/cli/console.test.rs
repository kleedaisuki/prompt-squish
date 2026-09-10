//! Module-local regression tests. / 紧邻模块的回归测试。
use super::*;

#[test]
fn color_preparse_respects_argument_boundary_and_forms() {
    for (args, expected) in [
        (
            vec!["xmlsquish", "--color=always", "--help"],
            ColorMode::Always,
        ),
        (
            vec!["xmlsquish", "--color", "never", "--bad"],
            ColorMode::Never,
        ),
        (vec!["xmlsquish", "--", "--color=always"], ColorMode::Auto),
        (vec!["xmlsquish", "--color=invalid"], ColorMode::Auto),
    ] {
        let args = args.into_iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(ColorMode::from_args(&args), expected);
    }
}
