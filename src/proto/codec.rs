use std::str::Utf8Error;

use capnp::{
    message::{self, ReaderOptions},
    serialize,
};
use rusqlite::types::Value;

use super::{
    ddcp_capnp::request,
    ddcp_capnp::{change_value, response},
};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{0}")]
    Capnp(#[from] capnp::Error),
    #[error("{0}")]
    NotInSchema(#[from] capnp::NotInSchema),
    #[error("utf-8 encoding error: {0}")]
    Utf8(#[from] std::str::Utf8Error),
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    Status,
    Changes { since_db_version: i64 },
}

#[derive(Debug, PartialEq)]
pub enum Response {
    Status {
        site_id: Vec<u8>,
        db_version: i64,
    },
    Changes {
        site_id: Vec<u8>,
        changes: Vec<Change>,
    },
}

#[derive(Debug, PartialEq, Clone)]
pub struct Change {
    pub table: String,
    pub pk: Vec<u8>,
    pub cid: String,
    pub val: Value,
    pub col_version: i64,
    pub db_version: i64,
    pub cl: i64,
    pub seq: i64,
}

pub fn encode_request_message(request: Request) -> Result<Vec<u8>> {
    let mut builder = message::Builder::new_default();
    let mut request_builder = builder.get_root::<request::Builder>()?;
    encode_request(request, &mut request_builder);
    Ok(serialize::write_message_segments_to_words(&builder))
}

pub fn decode_request_message(msg_bytes: &[u8]) -> Result<Request> {
    let reader = serialize::read_message(msg_bytes, ReaderOptions::new())?;
    let request_reader = reader.get_root::<request::Reader>()?;
    decode_request(&request_reader)
}

pub fn encode_request(request: Request, builder: &mut request::Builder) {
    match request {
        Request::Status => builder.set_status(()),
        Request::Changes { since_db_version } => builder
            .reborrow()
            .init_changes()
            .set_since_version(since_db_version),
    }
}

pub fn decode_request(reader: &request::Reader) -> Result<Request> {
    let which = reader.which()?;
    Ok(match which {
        request::Which::Status(_) => Request::Status,
        request::Which::Changes(Ok(changes)) => Request::Changes {
            since_db_version: changes.get_since_version(),
        },
        request::Which::Changes(Err(e)) => return Err(e.into()),
    })
}

pub fn encode_response_message(response: Response) -> Result<Vec<u8>> {
    let mut builder = message::Builder::new_default();
    let mut response_builder = builder.get_root::<response::Builder>()?;
    encode_response(response, &mut response_builder)?;
    Ok(serialize::write_message_segments_to_words(&builder))
}

pub fn decode_response_message(msg_bytes: &[u8]) -> Result<Response> {
    let reader = serialize::read_message(msg_bytes, ReaderOptions::new())?;
    let response_reader = reader.get_root::<response::Reader>()?;
    decode_response(&response_reader)
}

pub fn encode_response(response: Response, builder: &mut response::Builder) -> Result<()> {
    Ok(match response {
        Response::Changes { site_id, changes } => {
            let mut build_changes = builder.reborrow().init_changes();
            build_changes.set_site_id(&site_id);
            let mut build_changes_changes =
                build_changes.reborrow().init_changes(changes.len() as u32);
            for i in 0..changes.len() {
                let mut build_change = build_changes_changes.reborrow().get(i as u32);
                build_change.set_table(changes[i].table.as_str().into());
                build_change.set_pk(changes[i].pk.as_slice());
                build_change.set_cid(changes[i].cid.as_str().into());
                build_change.set_col_version(changes[i].col_version);
                build_change.set_db_version(changes[i].db_version);
                build_change.set_cl(changes[i].cl);
                build_change.set_seq(changes[i].seq);
                let mut val_change = build_change.reborrow().init_val();
                match &changes[i].val {
                    Value::Null => val_change.set_null(()),
                    Value::Integer(i) => val_change.set_integer(*i),
                    Value::Real(r) => val_change.set_real(*r),
                    Value::Text(t) => val_change.set_text(t.as_str().into()),
                    Value::Blob(b) => val_change.set_blob(b),
                };
            }
        }
        Response::Status {
            site_id,
            db_version,
        } => {
            let mut build_status = builder.reborrow().init_status();
            build_status.set_site_id(site_id.as_slice());
            build_status.set_db_version(db_version);
        }
    })
}

pub fn decode_response(reader: &response::Reader) -> Result<Response> {
    let which = reader.which()?;
    Ok(match which {
        response::Which::Status(Ok(status)) => Response::Status {
            db_version: status.get_db_version(),
            site_id: status.get_site_id()?.to_vec(),
        },
        response::Which::Status(Err(e)) => return Err(e.into()),
        response::Which::Changes(Ok(changes)) => {
            let changes_changes = changes.get_changes()?;
            let mut resp_changes = vec![];
            for i in 0..changes_changes.len() {
                let change = changes_changes.get(i);
                resp_changes.push(Change {
                    table: change.get_table()?.to_string()?,
                    pk: change.get_pk()?.to_vec(),
                    cid: change.get_cid()?.to_string()?,
                    val: match change.get_val()?.which()? {
                        change_value::Which::Null(_) => Value::Null,
                        change_value::Which::Integer(i) => Value::Integer(i),
                        change_value::Which::Real(r) => Value::Real(r),
                        change_value::Which::Text(t) => {
                            Value::Text(t?.to_string().map_err(|e| Utf8Error::from(e))?)
                        }
                        change_value::Which::Blob(b) => Value::Blob(b?.to_vec()),
                    },
                    col_version: change.get_col_version(),
                    db_version: change.get_db_version(),
                    cl: change.get_cl(),
                    seq: change.get_seq(),
                });
            }
            Response::Changes {
                site_id: changes.get_site_id()?.to_owned(),
                changes: resp_changes,
            }
        }
        response::Which::Changes(Err(e)) => return Err(e.into()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_status() {
        let msg_bytes = encode_request_message(Request::Status).expect("ok");
        let decoded = decode_request_message(&msg_bytes).expect("ok");
        assert_eq!(Request::Status, decoded);
    }

    #[test]
    fn response_status() {
        let msg_bytes = encode_response_message(Response::Status {
            site_id: "foo".as_bytes().to_vec(),
            db_version: 42,
        })
        .expect("ok");
        let decoded = decode_response_message(&msg_bytes).expect("ok");
        assert_eq!(
            Response::Status {
                site_id: "foo".as_bytes().to_vec(),
                db_version: 42
            },
            decoded
        );
    }

    #[test]
    fn request_changes() {
        let msg_bytes = encode_request_message(Request::Changes {
            since_db_version: 42,
        })
        .expect("ok");
        let decoded = decode_request_message(&msg_bytes).expect("ok");
        assert_eq!(
            Request::Changes {
                since_db_version: 42
            },
            decoded
        );
    }

    #[test]
    fn response_changes_empty() {
        let msg_bytes = encode_response_message(Response::Changes {
            site_id: "foo".as_bytes().to_vec(),
            changes: vec![],
        })
        .expect("ok");
        let decoded = decode_response_message(&msg_bytes).expect("ok");
        assert_eq!(
            Response::Changes {
                site_id: "foo".as_bytes().to_vec(),
                changes: vec![]
            },
            decoded
        );
    }

    #[test]
    fn response_changes() {
        let changes = vec![
            Change {
                table: "some_table".to_owned(),
                pk: "some_pk".to_string().into_bytes(),
                cid: "el_cid".to_owned(),
                val: rusqlite::types::Value::Integer(42),
                col_version: 23,
                db_version: 42,
                cl: 99,
                seq: 999,
            },
            Change {
                table: "some_other_table".to_owned(),
                pk: "some_other_pk".to_string().into_bytes(),
                cid: "a_cid".to_owned(),
                val: rusqlite::types::Value::Text("foo".to_string()),
                col_version: 32,
                db_version: 24,
                cl: 111,
                seq: 1111,
            },
        ];
        let msg_bytes = encode_response_message(Response::Changes {
            site_id: "foo".as_bytes().to_vec(),
            changes: changes.clone(),
        })
        .expect("ok");
        let decoded = decode_response_message(&msg_bytes).expect("ok");
        assert_eq!(
            Response::Changes {
                site_id: "foo".as_bytes().to_vec(),
                changes,
            },
            decoded
        );
    }
}
