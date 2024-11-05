#!/bin/bash
cp tests.sh tests-temp.sh
compare_gzip_outputs -v -k -c -f -1 tests.sh

compare_gzip_outputs() {
    # Capture all arguments passed to the function
    local args=("$@")

    # Create temporary files to store outputs
    GZIP_OUTPUT=$(mktemp)
    CARGO_OUTPUT=$(mktemp)

    # Run gzip with the provided arguments and capture stdout and stderr
    gzip "${args[@]}" > "$GZIP_OUTPUT" 2>&1

    # Run cargo run with the provided arguments and capture stdout and stderr
    cargo run --quiet -- "${args[@]}" > "$CARGO_OUTPUT" 2>&1

    # Compare the outputs
    if diff -u "$GZIP_OUTPUT" "$CARGO_OUTPUT"; then
        echo "Outputs are identical."
    else
        echo "Outputs differ."
    fi

    # Clean up temporary files
    rm "$GZIP_OUTPUT" "$CARGO_OUTPUT"
}