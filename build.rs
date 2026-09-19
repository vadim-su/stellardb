fn main() -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "cli")]
    build_metadata()?;

    #[cfg(feature = "grpc")]
    build_grpc()?;

    #[cfg(feature = "ui")]
    build_ui()?;

    Ok(())
}

#[cfg(feature = "cli")]
fn build_metadata() -> Result<(), Box<dyn std::error::Error>> {
    use vergen_gitcl::{BuildBuilder, CargoBuilder, Emitter, GitclBuilder, RustcBuilder};

    let build = BuildBuilder::default().build_timestamp(true).build()?;
    let cargo = CargoBuilder::default().build()?;
    let gitcl = GitclBuilder::default().sha(true).dirty(true).build()?;
    let rustc = RustcBuilder::default().semver(true).build()?;

    Emitter::default()
        .add_instructions(&build)?
        .add_instructions(&cargo)?
        .add_instructions(&gitcl)?
        .add_instructions(&rustc)?
        .emit()?;

    Ok(())
}

#[cfg(feature = "grpc")]
fn build_grpc() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo:rerun-if-changed=proto/stellar/v1");
    tonic_prost_build::configure()
        .build_server(true)
        .build_client(true)
        .compile_protos(
            &[
                "proto/stellar/v1/common.proto",
                "proto/stellar/v1/query.proto",
                "proto/stellar/v1/document.proto",
                "proto/stellar/v1/edge.proto",
                "proto/stellar/v1/database.proto",
                "proto/stellar/v1/metrics.proto",
            ],
            &["."],
        )?;

    Ok(())
}

#[cfg(feature = "ui")]
fn build_ui() -> Result<(), Box<dyn std::error::Error>> {
    use std::process::Command;

    let ui_dir = std::path::Path::new("station");

    // The published crate ships a prebuilt `station/dist` and no UI sources.
    // In that case there is nothing to build, and build.rs must not write into
    // the (read-only) registry source directory.
    if !ui_dir.join("package.json").exists() {
        if ui_dir.join("dist").join("index.html").exists() {
            return Ok(());
        }
        return Err("station/dist is missing and Station sources are not present".into());
    }

    // Rerun if UI source changes
    println!("cargo:rerun-if-changed=station/src");
    println!("cargo:rerun-if-changed=station/index.html");
    println!("cargo:rerun-if-changed=station/package.json");
    println!("cargo:rerun-if-changed=station/vite.config.ts");
    println!("cargo:rerun-if-changed=packages/client/src");
    println!("cargo:rerun-if-changed=packages/client/test");
    println!("cargo:rerun-if-changed=packages/client/package.json");
    println!("cargo:rerun-if-changed=packages/client/tsconfig.build.json");
    println!("cargo:rerun-if-changed=packages/client/tsconfig.test.json");

    // Check if bun is available
    let bun_check = Command::new("bun").arg("--version").output();
    if bun_check.is_err() || !bun_check.unwrap().status.success() {
        return Err("bun is not installed. Install it from https://bun.sh".into());
    }

    // Install dependencies if node_modules doesn't exist
    let node_modules = ui_dir.join("node_modules");
    if !node_modules.exists() {
        eprintln!("Installing Station dependencies...");
        let status = Command::new("bun")
            .arg("install")
            .current_dir(ui_dir)
            .status()?;
        if !status.success() {
            return Err("Failed to install UI dependencies".into());
        }
    }

    // Build UI
    eprintln!("Building Station...");
    let status = Command::new("bun")
        .args(["run", "build"])
        .current_dir(ui_dir)
        .status()?;
    if !status.success() {
        return Err("Failed to build Station".into());
    }

    Ok(())
}
