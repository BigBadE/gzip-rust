# Steps
- Ask for all globals in the file. This assumed the value of constants declared elsewhere, since I only passed a single file. Had to also specify that code should be platform independent, and should prefer safe code with no mutable static variables.
- Ask for each function to be translated individually. Went up the call graph.
    - Notable issues:
      - Stuck to C patterns instead of refactoring them, for example using exit() instead of returning out of the entire function
      - Duplicated globals
      - Ignored conditional compilation flags
      - ChatGPT would flat-out refuse to translate entire files, it may refuse large functions too
      - Variable types would change in global structures (better prompting would likely fix)
      - ChatGPT completely hallucinated the do_list function’s purpose, causing it to output a completely different function. This was fixed by passing in the entire function again, indicating memory is likely not that great
      - ChatGPT failed to understand connections between constants, for example messing up the name for each compression method from its id
      - ChatGPT stuck to the C method arguments when it shouldn't, leading to some borrow checker issues
    - Strengths:
      - ChatGPT knew to use external crates to fulfill platform-independent functionality, successfully adapting certain elements of gzip from linux-only methods to multi-platform ones
      - ChatGPT tended to stick to Rust paradigms when it was obvious, such as replacing i32 error codes with Results, though this didn’t always happen (better prompting may improve this more).
      - By providing the C code of the function at the time of translation as well as providing the entire file beforehand, hallucinations went down. My prompting was likely very suboptimal because I couldn't provide every single relevant Rust header and C snippet for every prompt

# Results:
Total: 1311 LOC
Accuracy for compilation: 96.6%
Manual fixes needed for simple (basic mismated types) errors: 17 lines
Manual fixes needed for more advanced errors: 28 lines

Note: This does not include bugs, given there were no tests. I manually tested it and found a few bugs, for example incorrect argument parsing ordering causing the version and license argument to not work
Also note: Rust auto-fixable warnings were applied for cleanliness (so I could see terminal output), the outputted code tended to ignore Results

Notable error types:
- Unspecified generic type when the generic type wasn't in the arguments, which more prompting fixed
- Mutable borrow on immutably borrowed type, though ChatGPT figured out an elegant fix with some more prompting
