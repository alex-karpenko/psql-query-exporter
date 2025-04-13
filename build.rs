use std::{env, process::Command};

const CERTIFICATES_GENERATOR_SCRIPT_FOLDER: &str = "tests/scripts";
const CERTIFICATES_GENERATOR_SCRIPT: &str = "create-test-certificates.sh";
const OPENSSL_CONFIG: &str = "openssl.cnf";

fn main() -> Result<(), anyhow::Error> {
    let out_dir = env::var("OUT_DIR")
        .expect("`OUT_DIR` environment variable isn`t set, use Cargo to run build");
    let dest_prefix = format!("{out_dir}/");

    let script_path =
        format!("{CERTIFICATES_GENERATOR_SCRIPT_FOLDER}/{CERTIFICATES_GENERATOR_SCRIPT}");
    let openssl_config_path = format!("{CERTIFICATES_GENERATOR_SCRIPT_FOLDER}/{OPENSSL_CONFIG}");

    Command::new(script_path.clone())
        .arg(dest_prefix)
        .status()?;

    println!("cargo::rerun-if-changed={script_path}");
    println!("cargo::rerun-if-changed={openssl_config_path}");
    println!("cargo::rerun-if-changed=build.rs");

    Ok(())
}
