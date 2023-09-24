@0xc9cb850366986f30;

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
        status @0 :Status;
        changes @1 :ChangesResult;
    }
}

struct ChangesResult {
    siteId @0 :Data;
    changes @1 :List(Change);
}

struct Status {
    siteId @0 :Data;
    dbVersion @1 :Int64;
}

struct Change {
    table @0 :Text;
    pk @1 :Data;
    cid @2 :Text;
    val @3 :Data;
    colVersion @4 :Int64;
    dbVersion @5 :Int64;
    siteId @6 :Data;
    cl @7 :Int64;
    seq @8 :Int64;
}
