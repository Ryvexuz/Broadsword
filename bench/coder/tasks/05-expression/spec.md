# Arithmetic expression evaluator

Write a function that parses and evaluates an integer arithmetic expression, with precedence, unary minus, parentheses, overflow detection and error positions.

## Public API

Your file must define exactly these public items (you may add private helpers and private types):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// A character that cannot appear anywhere in an expression (a letter, `$`, `.`, a non-ASCII char, ...).
    UnexpectedChar,
    /// The input ended where an operand or a closing parenthesis was still needed.
    UnexpectedEnd,
    /// A valid token in a place where it is not allowed.
    UnexpectedToken,
    /// An opening parenthesis that is never closed.
    UnclosedParen,
    /// Division or remainder by zero.
    DivisionByZero,
    /// A number or a result that does not fit in an `i64`.
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalError {
    pub kind: ErrorKind,
    /// 0-based position, counted in `char`s from the start of the input. See "Error positions".
    pub pos: usize,
}

pub fn evaluate(input: &str) -> Result<i64, EvalError>;
```

## Language

```
expr    = term   { ("+" | "-") term }
term    = unary  { ("*" | "/" | "%") unary }
unary   = "-" unary | primary
primary = NUMBER | "(" expr ")"
```

- `NUMBER` is one or more ASCII digits (`0`-`9`). Leading zeros are allowed. There is no decimal point and no exponent.
- Spaces and tabs may appear between any two tokens, and before the first and after the last. They are otherwise ignored. No other whitespace character is allowed (a newline is an `UnexpectedChar`).
- All operators are left-associative: `20 - 5 - 3` is 12, `64 / 4 / 2` is 8.
- `*`, `/`, `%` bind tighter than `+` and `-`. Unary minus binds tighter than all binary operators and may be repeated: `--4` is 4, `8 - -2` is 10, `3 * -6` is -18.
- There is no unary plus. A `+` where an operand is expected is an `UnexpectedToken`.
- Arithmetic is on `i64`. `/` truncates toward zero (`-9 / 4` is `-2`). `%` is the remainder of that division, with the sign of the left operand (`-9 % 4` is `-1`, `9 % -4` is `1`).
- An empty or all-whitespace input is `UnexpectedEnd` at position = the number of chars in the input.

## Overflow

- A number literal larger than `i64::MAX` (9223372036854775807) is an `Overflow`, with one exception: the literal `9223372036854775808` is accepted when it is the direct operand of a unary minus (whitespace between them is allowed), and the negated value is `i64::MIN`. Anywhere else (alone, after a binary operator as in `3 * 9223372036854775808`, or inside parentheses of their own as in `(0 + 9223372036854775808)`) it is an `Overflow`.
- The size rule for number literals is applied only when a number is read as an operand. A number in a place where an operand is not allowed (a huge number written right after another number) is an `UnexpectedToken`, whatever its size.
- Every `+`, `-`, `*`, `/`, `%` and unary minus must be checked. A result that does not fit in an `i64` is an `Overflow`. This includes negating `i64::MIN`, and `i64::MIN / -1` and `i64::MIN % -1`.
- Division or remainder with a right operand of 0 is `DivisionByZero`.

## Order in which errors are reported

1. **Syntax first.** The whole input is read from left to right, and checked against the language and the number literal rule above, before anything is evaluated. The first problem found while reading left to right is the one reported. An unclosed `(` is only noticed when the input ends, so any other problem to its right is found first. If the input is read to the end with an operand still missing, that is `UnexpectedEnd`, which is found before `UnclosedParen`.
2. **Then evaluation.** Only if the whole input is syntactically valid is it evaluated. Evaluate each operator after both of its operands have been evaluated, in left-to-right order, and report the first `Overflow` or `DivisionByZero` met. (So in `(8 / 0) + (3037000500 * 3037000500)` the division by zero is reported, because it is met first.)

## Error positions

`pos` is the 0-based char index of:

- `UnexpectedChar`: the offending character.
- `UnexpectedToken`: the first char of the offending token (an operator, a parenthesis, or the first digit of a number). Examples: a `)` with no open parenthesis; a `*`, `/` or `%` where an operand is expected (two binary operators in a row, or one at the very start); a number right after a number; a `(` right after an operand; a `+` where an operand is expected; a `)` directly after a `(`.
- `UnexpectedEnd`: the length of the input in chars.
- `UnclosedParen`: the position of the `(` that is never closed (when several are unclosed, the innermost, that is the last opened one).
- `DivisionByZero`: the position of the `/` or `%`.
- `Overflow`:
  - for a number literal: the position of its first digit;
  - for a binary `+ - * / %`: the position of the operator;
  - for a unary minus: the position of that `-`.

## Constraints

- Use only the Rust standard library. No external crates.
- No `unsafe` code.
- Input of any length and nesting depth up to 10,000 levels of parentheses must not crash the program.
- Provide a single complete Rust source file that compiles as a library crate root (`lib.rs`), edition 2021.
