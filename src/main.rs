use std::{io, path::PathBuf};

use rusqlite::{Connection, OptionalExtension};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("db error: {0}")]
    DB(#[from] rusqlite::Error),
    #[error("io error: {0}")]
    IO(#[from] io::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

fn main() {
    run().expect("ok");
}

fn run() -> Result<()> {
    let conn = Connection::open(":memory:")?;
    let ext_path = ext_path()?;
    unsafe {
        conn.load_extension_enable()?;
        let r = conn.load_extension(ext_path.as_str(), Some("sqlite3_crsqlite_init"))?;
        conn.load_extension_disable()?;
        r
    };
    
    // magic gonna happen here

    conn.execute("SELECT crsql_finalize()", []).optional()?;
    Ok(())
}

fn ext_path() -> Result<String> {
    let exe = std::env::current_exe()?;
    let exe_dir = exe.parent().expect("executable has a parent directory");
    Ok(String::from(
        exe_dir
            .join(PathBuf::from("crsqlite"))
            .as_os_str()
            .to_str()
            .expect("valid path string"),
    ))
}
