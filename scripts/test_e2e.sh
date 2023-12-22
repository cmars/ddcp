#!/usr/bin/env bash

set -eux

# cwd in project root
cd $(dirname $0)/..

test_dir=$(mktemp -d)
mkdir -p $test_dir/alice $test_dir/bob

trap "rm -rf $test_dir" EXIT

fixtures=$(pwd)/fixtures
ddcp=$(pwd)/target/release/ddcp
cd ${test_dir}

# Create alice and bob databases & publish them
echo "Creating Alice db"
alice_addr=$(env DB_FILE=alice/db $ddcp addr)
echo "Alice addr: $alice_addr"
env DB_FILE=alice/db $ddcp shell < $fixtures/cr_test_schema.sql
env DB_FILE=alice/db $ddcp shell < $fixtures/ins_test_schema_alice.sql

echo "Creating Bob db"
bob_addr=$(env DB_FILE=bob/db $ddcp addr)
echo "Bob addr: $bob_addr"
env DB_FILE=bob/db $ddcp shell < $fixtures/cr_test_schema.sql
env DB_FILE=bob/db $ddcp shell < $fixtures/ins_test_schema_bob.sql

echo "Setting up remotes"
env DB_FILE=alice/db $ddcp remote add bob ${bob_addr}
env DB_FILE=bob/db $ddcp remote add alice ${alice_addr}

echo "Start Alice server"
env DB_FILE=alice/db RUST_LOG=none,ddcp=debug $ddcp serve &
alice_pid=$!

sleep 15

echo "Start Bob server"
env DB_FILE=bob/db RUST_LOG=none,ddcp=debug $ddcp serve &
bob_pid=$!

trap "rm -rf $test_dir; kill $alice_pid; kill $bob_pid" EXIT

sleep 15

for i in {15..30}; do
    alice_expect_row_count=$(sqlite3 alice/db "select count(*) from posts where contents like 'Look at this really cool project%'")
    bob_expect_row_count=$(sqlite3 bob/db "select count(*) from profiles where about = 'The O.G. metasyntactial protocol practitioner'")
    if [[ "$alice_expect_row_count" = "1" && "$bob_expect_row_count" = "1" ]]; then
        echo "PASS: matched expected rows in both databases"
        exit 0
    fi
    sleep $i
done

echo "FAIL: sync never detected"
exit 1
