use crate::{Result};
use shiguredo_toml::{Document as TomlDocument, Value as TomlValue};

pub(crate) fn rewrite_cargo_config_toml(
    input: &str,
    rust_target: &str,
    linker: &str,
    sysroot_arg: &str,
    cc_value: &str,
    cxx_value: &str,
) -> Result<String> {
    let mut doc = TomlDocument::parse(input)?;
    let target_prefix = format!("target.{rust_target}");
    doc.set_path(
        &format!("{target_prefix}.linker"),
        TomlValue::String(linker.to_string()),
    )?;
    doc.set_path(
        &format!("{target_prefix}.rustflags"),
        TomlValue::Array(vec![
            TomlValue::String("-C".to_string()),
            TomlValue::String(sysroot_arg.to_string()),
        ]),
    )?;

    let target_key = rust_target.replace('-', "_");
    let cc_key = format!("CC_{target_key}");
    let cxx_key = format!("CXX_{target_key}");

    doc.set_path(
        &format!("env.{cc_key}"),
        relative_env_value(cc_value),
    )?;
    doc.set_path(
        &format!("env.{cxx_key}"),
        relative_env_value(cxx_value),
    )?;

    Ok(doc.as_str().to_string())
}

fn relative_env_value(value: &str) -> TomlValue {
    let mut table = shiguredo_toml::Table::new();
    table.insert("relative".to_string(), TomlValue::Boolean(true));
    table.insert("value".to_string(), TomlValue::String(value.to_string()));
    TomlValue::Table(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn rewrite_cargo_config_toml_create() {
        let output = rewrite_cargo_config_toml(
            "",
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect("rewrite config");
        let expected = r#"[env]
CC_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cc-wrapper.sh"}
CXX_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cxx-wrapper.sh"}
[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
rustflags = ["-C", "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot"]
"#;
        assert_eq!(output, expected);
    }

    #[test]
    fn rewrite_cargo_config_toml_update_keep_other_keys() {
        let input = r#"[target.aarch64-unknown-linux-gnu]
linker = "old"
rustflags = ["-C", "old"]
foo = "bar"

[env]
FOO = "BAR"
CC_aarch64_unknown_linux_gnu = "old-cc"
CFLAGS_aarch64_unknown_linux_gnu = "old-cflags"
CXXFLAGS_aarch64_unknown_linux_gnu = "old-cxxflags"
"#;
        let output = rewrite_cargo_config_toml(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect("rewrite config");
        let expected = r#"[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc"
rustflags = ["-C", "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot"]
foo = "bar"

[env]
FOO = "BAR"
CC_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cc-wrapper.sh"}
CFLAGS_aarch64_unknown_linux_gnu = "old-cflags"
CXXFLAGS_aarch64_unknown_linux_gnu = "old-cxxflags"
CXX_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cxx-wrapper.sh"}
"#;
        assert_eq!(output, expected);
    }

    #[test]
    fn rewrite_cargo_config_toml_preserve_unrelated_lines_on_existing_file() {
        let input = r#"[env]
FOO = "BAR" # keep
CC_aarch64_unknown_linux_gnu = "old-cc" # keep comment
CFLAGS_aarch64_unknown_linux_gnu = "old-cflags"

[target.aarch64-unknown-linux-gnu]
linker = "old-linker" # keep spacing
rustflags = ["-C", "old"] # keep
other = "stay" # keep
"#;
        let output = rewrite_cargo_config_toml(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect("rewrite config");
        let expected = r#"[env]
FOO = "BAR" # keep
CC_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cc-wrapper.sh"} # keep comment
CFLAGS_aarch64_unknown_linux_gnu = "old-cflags"
CXX_aarch64_unknown_linux_gnu = {relative = true, value = "../target/cxx-wrapper.sh"}

[target.aarch64-unknown-linux-gnu]
linker = "aarch64-linux-gnu-gcc" # keep spacing
rustflags = ["-C", "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot"] # keep
other = "stay" # keep
"#;
        assert_eq!(output, expected);
    }

    #[test]
    fn rewrite_cargo_config_toml_invalid_input_fails() {
        let input = "[target\nbroken = true";
        assert!(
            rewrite_cargo_config_toml(
                input,
                "aarch64-unknown-linux-gnu",
                "aarch64-linux-gnu-gcc",
                "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
                "../target/cc-wrapper.sh",
                "../target/cxx-wrapper.sh",
            )
            .is_err()
        );
    }

    #[test]
    fn rewrite_cargo_config_toml_type_conflict_fails() {
        let input = r#"target = "bad""#;
        assert!(
            rewrite_cargo_config_toml(
                input,
                "aarch64-unknown-linux-gnu",
                "aarch64-linux-gnu-gcc",
                "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
                "../target/cc-wrapper.sh",
                "../target/cxx-wrapper.sh",
            )
            .is_err()
        );
    }

    #[test]
    fn rewrite_cargo_config_toml_env_not_table_fails() {
        let input = "env = \"bad\"\n";
        let err = rewrite_cargo_config_toml(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect_err("env が table でない場合は失敗するべき");
        assert!(
            err.to_string()
                .contains("parent path does not point to a table"),
            "想定外のエラー内容: {err}"
        );
    }

    #[test]
    fn rewrite_cargo_config_toml_target_triple_not_table_fails() {
        let input = r#"[target]
aarch64-unknown-linux-gnu = "bad"
"#;
        let err = rewrite_cargo_config_toml(
            input,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect_err("target.<triple> が table でない場合は失敗するべき");
        assert!(
            err.to_string()
                .contains("parent path does not point to a table"),
            "想定外のエラー内容: {err}"
        );
    }

    #[test]
    fn rewrite_cargo_config_toml_idempotent() {
        let once = rewrite_cargo_config_toml(
            "",
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect("rewrite config");
        let twice = rewrite_cargo_config_toml(
            &once,
            "aarch64-unknown-linux-gnu",
            "aarch64-linux-gnu-gcc",
            "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
            "../target/cc-wrapper.sh",
            "../target/cxx-wrapper.sh",
        )
        .expect("rewrite config");
        assert_eq!(once, twice);
    }

    proptest! {
        #[test]
        fn rewrite_cargo_config_toml_is_idempotent_for_valid_input(input in ".*") {
            let once = rewrite_cargo_config_toml(
                &input,
                "aarch64-unknown-linux-gnu",
                "aarch64-linux-gnu-gcc",
                "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
                "../target/cc-wrapper.sh",
                "../target/cxx-wrapper.sh",
            );
            if let Ok(once) = once {
                let twice = rewrite_cargo_config_toml(
                    &once,
                    "aarch64-unknown-linux-gnu",
                    "aarch64-linux-gnu-gcc",
                    "link-arg=--sysroot=target/shiguredo-sysroot/ubuntu-24.04_armv8/sysroot",
                    "../target/cc-wrapper.sh",
                    "../target/cxx-wrapper.sh",
                ).expect("rewrite config");
                prop_assert_eq!(once, twice);
            }
        }
    }
}
