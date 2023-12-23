use std::process::Command;

fn main() {
    // Compile cr-sqlite
    let build_crsqlite_output = Command::new("bash")
        .args(["scripts/build_cr_sqlite.sh"])
        .output()
        .expect("failed to build cr-sqlite");
    if !build_crsqlite_output.status.success() {
        panic!(
            "{}\n{}",
            String::from_utf8(build_crsqlite_output.stdout).expect("utf8"),
            String::from_utf8(build_crsqlite_output.stderr).expect("utf8")
        );
    }

    // Compile Capn'Proto
    ::capnpc::CompilerCommand::new()
        .output_path("src/")
        .default_parent_module(vec!["proto".into()])
        .file("proto/ddcp.capnp")
        .run()
        .expect("compiling schema");
}
