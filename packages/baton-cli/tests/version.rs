use std::process::Command;

#[test]
fn version_options_print_version_without_initializing_a_project() {
    for option in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_baton"))
            .arg(option)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("baton {}\n", env!("CARGO_PKG_VERSION"))
        );
        assert!(output.stderr.is_empty());
    }
}
