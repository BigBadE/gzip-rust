#!/bin/bash

passed=0
total=0

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
    mv "$file_name.gz" "$GZIP_OUTPUT_FILE" > /dev/null 2>&1

    # Run cargo run with the provided arguments and capture stdout and stderr
    cargo build > /dev/null 2>&1
    ./target/debug/gzip "${args[@]}" > "$CARGO_OUTPUT" 2>&1
    mv "$file_name.gz" "$CARGO_OUTPUT_FILE" > /dev/null 2>&1

    # Compare the outputs
    if diff -u "$GZIP_OUTPUT" "$CARGO_OUTPUT" && diff -u "$GZIP_OUTPUT_FILE" "$CARGO_OUTPUT_FILE"; then
        echo "Test passed."
        ((passed++))
    else
        echo "Test failed."
        mv "$GZIP_OUTPUT" "target/gzip_console_output.txt"
        mv "$CARGO_OUTPUT" "target/cargo_console_output.txt"
        mv "$GZIP_OUTPUT_FILE" "target/gzip_output.gz"
        mv "$CARGO_OUTPUT_FILE" "target/cargo_output.gz"
    fi

    # Clean up temporary files
    rm "$GZIP_OUTPUT" "$GZIP_OUTPUT_FILE" "$CARGO_OUTPUT" "$CARGO_OUTPUT_FILE" > /dev/null 2>&1
    ((total++))
}

compare_gzip_outputs_no_file() {
    # Capture all arguments passed to the function
    local args=("$@")

    # Create temporary files to store outputs
    GZIP_OUTPUT=$(mktemp)
    CARGO_OUTPUT=$(mktemp)

    # Run gzip with the provided arguments and capture stdout and stderr
    gzip "${args[@]}" > "$GZIP_OUTPUT" 2>&1

    # Run cargo run with the provided arguments and capture stdout and stderr
    cargo build > /dev/null 2>&1
    ./target/debug/gzip "${args[@]}" > "$CARGO_OUTPUT" 2>&1

    # Compare the outputs
    if diff -u "$GZIP_OUTPUT" "$CARGO_OUTPUT"; then
        echo "Test passed."
        ((passed++))
    else
        echo "Test failed. Wrote failed files to target"
        mv "$GZIP_OUTPUT" "target/gzip_console_output.txt"
        mv "$CARGO_OUTPUT" "target/cargo_console_output.txt"
    fi

    # Clean up temporary files
    rm "$GZIP_OUTPUT" "$CARGO_OUTPUT" > /dev/null 2>&1
    ((total++))
}

echo "Testing no-arg output"
compare_gzip_outputs_no_file " "

echo "Testing nonexistant"
compare_gzip_outputs -k -1 test.txt

echo "Testing already existing output"
touch tests/test-word.txt.gz
#compare_gzip_outputs -k -1 tests/test-word.txt

echo "Testing forced overwrite"
compare_gzip_outputs -k -f -1 tests/test-word.txt

echo "Testing delete"
touch tests/test-temp.txt
compare_gzip_outputs -f -1 tests/test-temp.txt
echo "Testing file is deleted"
if [ -f tests/test-temp.txt ]; then
  echo "Test failed. File not deleted"
else
  echo "Test passed."
  ((passed++))
fi
((total++))

echo "Testing help menu"
compare_gzip_outputs_no_file -h

echo "Testing empty bits operand"
compare_gzip_outputs_no_file -b

echo "Testing incorrect bits operand"
compare_gzip_outputs_no_file -b test

echo "Testing compression level 2"
compare_gzip_outputs -k -2 tests/test-word.txt

echo "Testing compression level 3"
compare_gzip_outputs -k -3 tests/test-word.txt

echo "Testing ascii mode"
compare_gzip_outputs -k -a -1 tests/test-word.txt

echo "Testing stdout mode"
compare_gzip_outputs -k -c -1 tests/test-word.txt

echo "Testing quiet mode"
compare_gzip_outputs -k -q -1 tests/test-word.txt

echo "Testing verbose mode"
compare_gzip_outputs -v -k -f -1 tests/test-word.txt

echo "Testing version"
compare_gzip_outputs_no_file -L

echo "Testing test files"
for file in tests/*; do
  echo "Testing $file"
  compare_gzip_outputs -k -f -1 "$file"
done

echo "Passed: $passed out of $total"