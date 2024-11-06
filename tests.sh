#!/bin/bash

compare_gzip_outputs() {
    # Capture all arguments passed to the function
    local args=("$@")
    file_name="${args[-1]}"

    # Create temporary files to store outputs
    GZIP_OUTPUT=$(mktemp)
    GZIP_OUTPUT_FILE=$(mktemp)
    CARGO_OUTPUT=$(mktemp)
    CARGO_OUTPUT_FILE=$(mktemp)

    # Run gzip with the provided arguments and capture stdout and stderr
    gzip "${args[@]}" > "$GZIP_OUTPUT" 2>&1
    cp "$file_name.gz" "$GZIP_OUTPUT_FILE" > /dev/null 2>&1

    # Run cargo run with the provided arguments and capture stdout and stderr
    cargo build > /dev/null 2>&1
    ./target/debug/gzip "${args[@]}" > "$CARGO_OUTPUT" 2>&1
    mv "$file_name.gz" "$CARGO_OUTPUT_FILE" > /dev/null 2>&1

    # Compare the outputs
    if diff -u "$GZIP_OUTPUT" "$CARGO_OUTPUT" && diff -u "$GZIP_OUTPUT_FILE" "$CARGO_OUTPUT_FILE"; then
        echo "Test passed."
    else
        echo "Test failed. Wrote failed files to target"
        mv "$GZIP_OUTPUT" "target/gzip_console_output.txt"
        mv "$CARGO_OUTPUT" "target/cargo_console_output.txt"
        mv "$GZIP_OUTPUT_FILE" "target/gzip_output.gz"
        mv "$CARGO_OUTPUT_FILE" "target/cargo_output.gz"
    fi

    # Clean up temporary files
    rm "$GZIP_OUTPUT" "$GZIP_OUTPUT_FILE" "$CARGO_OUTPUT" "$CARGO_OUTPUT_FILE" > /dev/null 2>&1
}

echo "Testing help menu"
compare_gzip_outputs -h test.txt

echo "Testing test files"
for file in tests/*; do
  echo "Testing $file"
  compare_gzip_outputs -k -f -1 "$file"
done
