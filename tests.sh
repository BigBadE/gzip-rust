cp tests.sh tests-temp.sh
cargo run -- -v -f -1 tests-temp.sh

# Compare tests-temp.sh.gz to test-temp-official.sh.gz