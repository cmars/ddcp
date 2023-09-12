use std::process::Command;

fn main() {
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
}
