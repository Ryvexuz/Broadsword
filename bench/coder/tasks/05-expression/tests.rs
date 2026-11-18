mod common;
use common::within;
use solution::ErrorKind::*;
use solution::*;

fn ev(s: &str) -> Result<i64, EvalError> {
    let s = s.to_string();
    within(move || evaluate(&s))
}

fn e(kind: ErrorKind, pos: usize) -> Result<i64, EvalError> {
    Err(EvalError { kind, pos })
}

fn assert_ok(s: &str, want: i64) {
    assert_eq!(ev(s), Ok(want), "input {s:?}");
}

fn assert_err(s: &str, kind: ErrorKind, pos: usize) {
    assert_eq!(ev(s), e(kind, pos), "input {s:?}");
}

#[test]
fn hidden_numbers_and_whitespace() {
    assert_ok("0", 0);
    assert_ok("7", 7);
    assert_ok("007", 7);
    assert_ok("  42  ", 42);
    assert_ok("\t1\t+\t2\t", 3);
    assert_ok(" 1 + 2 ", 3);
    assert_ok("0009223372036854775807", i64::MAX);
}

#[test]
fn hidden_precedence() {
    assert_ok("1+2*3", 7);
    assert_ok("2*3+4*5", 26);
    assert_ok("2+3*4-5", 9);
    assert_ok("10-4%3", 9);
    assert_ok("10%4*3", 6);
    assert_ok("1+8/2/2", 3);
    assert_ok("2*3%4", 2);
}

#[test]
fn hidden_left_associativity() {
    assert_ok("10-3-2", 5);
    assert_ok("100/10/5", 2);
    assert_ok("8/2/2", 2);
    assert_ok("2-3+4", 3);
    assert_ok("20%7%4", 2);
    assert_ok("24/4*2", 12);
    assert_ok("24/(4*2)", 3);
}

#[test]
fn hidden_parentheses() {
    assert_ok("(1+2)*3", 9);
    assert_ok("2*(3+4)*5", 70);
    assert_ok("((((5))))", 5);
    assert_ok("(2)", 2);
    assert_ok("(1+(2*(3+(4*5))))", 47);
    assert_ok("( 1 + 2 ) * ( 3 )", 9);
}

#[test]
fn hidden_unary_minus() {
    assert_ok("-5", -5);
    assert_ok("- 2", -2);
    assert_ok("--3", 3);
    assert_ok("---3", -3);
    assert_ok("- -3", 3);
    assert_ok("2--3", 5);
    assert_ok("2*-3", -6);
    assert_ok("2 - -3", 5);
    assert_ok("-(2+3)", -5);
    assert_ok("-(-(3))", 3);
    assert_ok("-0", 0);
    assert_ok("-2*3", -6);
    assert_ok("-2+3", 1);
    assert_ok("3+-2*-2", 7);
    assert_ok("-(1+2)*-(3)", 9);
}

#[test]
fn hidden_division_and_remainder_signs() {
    assert_ok("7/2", 3);
    assert_ok("-7/2", -3);
    assert_ok("7/-2", -3);
    assert_ok("-7/-2", 3);
    assert_ok("7%2", 1);
    assert_ok("-7%2", -1);
    assert_ok("7%-2", 1);
    assert_ok("-7%-2", -1);
    assert_ok("0/5", 0);
    assert_ok("0%5", 0);
}

#[test]
fn hidden_extreme_values_that_fit() {
    assert_ok("9223372036854775807", i64::MAX);
    assert_ok("-9223372036854775808", i64::MIN);
    assert_ok("- 9223372036854775808", i64::MIN);
    assert_ok("-0009223372036854775808", i64::MIN);
    assert_ok("-9223372036854775807-1", i64::MIN);
    assert_ok("-9223372036854775808+1", i64::MIN + 1);
    assert_ok("9223372036854775806+1", i64::MAX);
    assert_ok("(-9223372036854775807-1)+9223372036854775807", -1);
    assert_ok("9223372036854775807*-1", -i64::MAX);
    assert_ok("-4611686018427387904*2", i64::MIN);
    assert_ok("-9223372036854775808/1", i64::MIN);
    assert_ok("-9223372036854775808%1", 0);
    assert_ok("-9223372036854775808/-2", 4611686018427387904);
}

#[test]
fn hidden_end_of_input_errors() {
    assert_err("", UnexpectedEnd, 0);
    assert_err("   ", UnexpectedEnd, 3);
    assert_err("1+", UnexpectedEnd, 2);
    assert_err("1 + ", UnexpectedEnd, 4);
    assert_err("2*", UnexpectedEnd, 2);
    assert_err("-", UnexpectedEnd, 1);
    assert_err("(", UnexpectedEnd, 1);
    assert_err("((", UnexpectedEnd, 2);
    assert_err("1 + (", UnexpectedEnd, 5);
    assert_err("(1 +", UnexpectedEnd, 4);
    assert_err("-\t-", UnexpectedEnd, 3);
}

#[test]
fn hidden_unclosed_paren() {
    assert_err("(1", UnclosedParen, 0);
    assert_err("((1", UnclosedParen, 1);
    assert_err("(1+2", UnclosedParen, 0);
    assert_err("2*(3+4", UnclosedParen, 2);
    assert_err("(1 + (2)", UnclosedParen, 0);
    assert_err("((1) + (2", UnclosedParen, 7);
}

#[test]
fn hidden_unexpected_token() {
    assert_err(")", UnexpectedToken, 0);
    assert_err("1)", UnexpectedToken, 1);
    assert_err("()", UnexpectedToken, 1);
    assert_err("( )", UnexpectedToken, 2);
    assert_err("(1))", UnexpectedToken, 3);
    assert_err("1 2", UnexpectedToken, 2);
    assert_err("2 (3)", UnexpectedToken, 2);
    assert_err("(2)(3)", UnexpectedToken, 3);
    assert_err("(2) 3", UnexpectedToken, 4);
    assert_err("+5", UnexpectedToken, 0);
    assert_err("2*+3", UnexpectedToken, 2);
    assert_err("* 3", UnexpectedToken, 0);
    assert_err("2 * * 3", UnexpectedToken, 4);
    assert_err("2*/3", UnexpectedToken, 2);
    assert_err("1 + + 2", UnexpectedToken, 4);
    assert_err("1 99999999999999999999", UnexpectedToken, 2);
    assert_err("-*2", UnexpectedToken, 1);
    assert_err("%", UnexpectedToken, 0);
}

#[test]
fn hidden_unexpected_char() {
    assert_err("a", UnexpectedChar, 0);
    assert_err("1 $ 2", UnexpectedChar, 2);
    assert_err("1.5", UnexpectedChar, 1);
    assert_err("1,2", UnexpectedChar, 1);
    assert_err("1 +\n2", UnexpectedChar, 3);
    assert_err("2 x 3", UnexpectedChar, 2);
    assert_err("1+\u{e9}", UnexpectedChar, 2);
    assert_err("\u{e9}1", UnexpectedChar, 0);
    assert_err("1 + 2 ^ 3", UnexpectedChar, 6);
    assert_err("$ )", UnexpectedChar, 0);
    assert_err("1 +\r\n2", UnexpectedChar, 3);
}

#[test]
fn hidden_syntax_errors_come_in_left_to_right_order() {
    assert_err("1 2 $", UnexpectedToken, 2);
    assert_err("(1 $", UnexpectedChar, 3);
    assert_err("1/0 + $", UnexpectedChar, 6);
    assert_err("9223372036854775807+1 $", UnexpectedChar, 22);
    assert_err("99999999999999999999 99999999999999999999", Overflow, 0);
    assert_err("(99999999999999999999", Overflow, 1);
    assert_err("1 + * 99999999999999999999", UnexpectedToken, 4);
}

#[test]
fn hidden_division_by_zero() {
    assert_err("1/0", DivisionByZero, 1);
    assert_err("1 / 0", DivisionByZero, 2);
    assert_err("5 % 0", DivisionByZero, 2);
    assert_err("0/0", DivisionByZero, 1);
    assert_err("5 / (3 - 3)", DivisionByZero, 2);
    assert_err("10 / 2 / 0", DivisionByZero, 7);
    assert_err("-5 / -0", DivisionByZero, 3);
    assert_err("(1/0)+(9223372036854775807*2)", DivisionByZero, 2);
}

#[test]
fn hidden_literal_overflow() {
    assert_err("9223372036854775808", Overflow, 0);
    assert_err("99999999999999999999999999", Overflow, 0);
    assert_err("2 - 9223372036854775808", Overflow, 4);
    assert_err("-(9223372036854775808)", Overflow, 2);
    assert_err("(9223372036854775808)", Overflow, 1);
    assert_err("-9223372036854775809", Overflow, 1);
    assert_err("1 + 18446744073709551616", Overflow, 4);
    assert_err("0009223372036854775808", Overflow, 0);
    assert_err("2*9223372036854775808", Overflow, 2);
}

#[test]
fn hidden_arithmetic_overflow() {
    assert_err("9223372036854775807 + 1", Overflow, 20);
    assert_err("-9223372036854775807 - 2", Overflow, 21);
    assert_err("0-9223372036854775807-2", Overflow, 21);
    assert_err("9223372036854775807 * 2", Overflow, 20);
    assert_err("4611686018427387904 * 2", Overflow, 20);
    assert_err("-4611686018427387905*2", Overflow, 20);
    assert_err("-9223372036854775808 - 1", Overflow, 21);
    assert_err("-9223372036854775808+-1", Overflow, 20);
}

#[test]
fn hidden_unary_minus_overflow() {
    assert_err("-(-9223372036854775807-1)", Overflow, 0);
    assert_err("--9223372036854775808", Overflow, 0);
    assert_err("1+-(-9223372036854775807-1)", Overflow, 2);
}

#[test]
fn hidden_min_divided_by_minus_one() {
    assert_err("(-9223372036854775807-1)/-1", Overflow, 24);
    assert_err("(-9223372036854775807-1)%-1", Overflow, 24);
    assert_err("(-9223372036854775807-1)*-1", Overflow, 24);
    assert_err("-9223372036854775808/-1", Overflow, 20);
}

#[test]
fn hidden_evaluation_order_is_operands_first_left_to_right() {
    assert_err("(9223372036854775807*2)+(1/0)", Overflow, 20);
    assert_err("9223372036854775807*2/0", Overflow, 19);
    assert_err("1/0*9223372036854775807*2", DivisionByZero, 1);
    assert_err("(9223372036854775807+1)/0", Overflow, 20);
    assert_err("5/0 + -(-9223372036854775807-1)", DivisionByZero, 1);
}

#[test]
fn hidden_deep_nesting() {
    let r = within(|| {
        let n = 10_000;
        let s = "(".repeat(n) + "1" + &")".repeat(n);
        evaluate(&s)
    });
    assert_eq!(r, Ok(1));
    let r = within(|| {
        let s = "(".repeat(10_000) + "1";
        evaluate(&s)
    });
    assert_eq!(r, e(UnclosedParen, 9_999));
    let r = within(|| {
        let s = "(".repeat(5_000) + "1" + &")".repeat(5_001);
        evaluate(&s)
    });
    assert_eq!(r, e(UnexpectedToken, 10_001));
}

#[test]
fn hidden_long_inputs() {
    let r = within(|| {
        let s = vec!["1"; 100_000].join("+");
        evaluate(&s)
    });
    assert_eq!(r, Ok(100_000));
    let r = within(|| evaluate(&("-".repeat(100_000) + "1")));
    assert_eq!(r, Ok(1));
    let r = within(|| evaluate(&("-".repeat(100_001) + "1")));
    assert_eq!(r, Ok(-1));
    let r = within(|| evaluate(&(" ".repeat(100_000) + "5")));
    assert_eq!(r, Ok(5));
    let r = within(|| evaluate(&("1".repeat(100_000))));
    assert_eq!(r, e(Overflow, 0));
}
