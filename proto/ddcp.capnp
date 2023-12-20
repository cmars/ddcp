@0xc9cb850366986f30;

struct Envelope {
    sender @0 :Text;
    contents @1 :Data;
}

struct Request {
    union {
        status @0 :Void;
        changes @1 :ChangesParams;
    }
}

struct ChangesParams {
    sinceVersion @0 :Int64;
}

struct Response {
    union {
        status @0 :DBStatus;
        changes @1 :ChangesResult;
    }
}

struct ChangesResult {
    siteId @0 :Data;
    changes @1 :List(Change);
}

struct DBStatus {
    siteId @0 :Data;
    dbVersion @1 :Int64;
}

struct NodeStatus {
    db @0 :DBStatus;
    key @1 :Text;
    route @2 :Data;
}

struct Change {
    table @0 :Text;
    pk @1 :Data;
    cid @2 :Text;
    val @3 :ChangeValue;
    colVersion @4 :Int64;
    dbVersion @5 :Int64;
    siteId @6 :Data;
    cl @7 :Int64;
    seq @8 :Int64;
}

struct ChangeValue {
    union {
        null @0 :Void;
        integer @1 :Int64;
        real @2 :Float64;
        text @3 :Text;
        blob @4 :Data;
    }
}
