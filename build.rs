use std::process::Command;

fn main() {
    // Compile cr-sqlite
    let output = Command::new("sh")
        .args(["build_cr_sqlite.sh"])
        .output()
        .expect("failed to build cr-sqlite");
    if !output.status.success() {
        panic!(
            "{}\n{}",
            String::from_utf8(output.stdout).expect("utf8"),
            String::from_utf8(output.stderr).expect("utf8")
        );
    }

    // Compile Capn'Proto
    ::capnpc::CompilerCommand::new()
        .output_path("src/")
        .default_parent_module(vec!["proto".into()])
        .file("proto/velouria.capnp")
        .run()
        .expect("compiling schema");
}
