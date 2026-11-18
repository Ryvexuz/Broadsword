#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    UnexpectedChar,
    UnexpectedEnd,
    UnexpectedToken,
    UnclosedParen,
    DivisionByZero,
    Overflow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalError {
    pub kind: ErrorKind,
    pub pos: usize,
}

fn err<T>(kind: ErrorKind, pos: usize) -> Result<T, EvalError> {
    Err(EvalError { kind, pos })
}

#[derive(Clone, Copy)]
enum Tok {
    Num(u128), // saturated: anything above 2^64 is stored as 2^64
    Op(char),
    LParen,
    RParen,
    Bad,
    End,
}

const TWO_63: u128 = 1u128 << 63;

fn lex(chars: &[char]) -> Vec<(usize, Tok)> {
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => i += 1,
            '0'..='9' => {
                let start = i;
                let mut v: u128 = 0;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    v = (v * 10 + chars[i].to_digit(10).unwrap() as u128).min(1u128 << 64);
                    i += 1;
                }
                toks.push((start, Tok::Num(v)));
            }
            '+' | '-' | '*' | '/' | '%' => {
                toks.push((i, Tok::Op(c)));
                i += 1;
            }
            '(' => {
                toks.push((i, Tok::LParen));
                i += 1;
            }
            ')' => {
                toks.push((i, Tok::RParen));
                i += 1;
            }
            _ => {
                toks.push((i, Tok::Bad));
                i += 1;
            }
        }
    }
    toks.push((chars.len(), Tok::End));
    toks
}

enum Item {
    Push(i64),
    Neg(usize),
    Bin(char, usize),
}

struct Parser {
    toks: Vec<(usize, Tok)>,
    at: usize,
    out: Vec<Item>,
}

impl Parser {
    fn peek(&self) -> (usize, Tok) {
        self.toks[self.at]
    }

    fn advance(&mut self) {
        if self.at + 1 < self.toks.len() {
            self.at += 1;
        }
    }

    fn expr(&mut self, depth: usize) -> Result<(), EvalError> {
        self.term(depth)?;
        loop {
            match self.peek() {
                (pos, Tok::Op(c)) if c == '+' || c == '-' => {
                    self.advance();
                    self.term(depth)?;
                    self.out.push(Item::Bin(c, pos));
                }
                _ => return self.after_operand(depth),
            }
        }
    }

    fn term(&mut self, depth: usize) -> Result<(), EvalError> {
        self.unary(depth)?;
        loop {
            match self.peek() {
                (pos, Tok::Op(c)) if c == '*' || c == '/' || c == '%' => {
                    self.advance();
                    self.unary(depth)?;
                    self.out.push(Item::Bin(c, pos));
                }
                _ => return Ok(()),
            }
        }
    }

    // Called when the next token cannot continue the expression: it must be a legal end.
    fn after_operand(&mut self, depth: usize) -> Result<(), EvalError> {
        match self.peek() {
            (_, Tok::End) => Ok(()),
            (_, Tok::RParen) if depth > 0 => Ok(()),
            (pos, Tok::Bad) => err(ErrorKind::UnexpectedChar, pos),
            (pos, _) => err(ErrorKind::UnexpectedToken, pos),
        }
    }

    fn unary(&mut self, depth: usize) -> Result<(), EvalError> {
        let mut negs: Vec<usize> = Vec::new();
        while let (pos, Tok::Op('-')) = self.peek() {
            negs.push(pos);
            self.advance();
        }
        let (pos, tok) = self.peek();
        match tok {
            Tok::Num(v) => {
                self.advance();
                if v > TWO_63 {
                    return err(ErrorKind::Overflow, pos);
                }
                if v == TWO_63 {
                    if negs.pop().is_none() {
                        return err(ErrorKind::Overflow, pos);
                    }
                    self.out.push(Item::Push(i64::MIN));
                } else {
                    self.out.push(Item::Push(v as i64));
                }
            }
            Tok::LParen => {
                self.advance();
                self.expr(depth + 1)?;
                match self.peek() {
                    (_, Tok::RParen) => self.advance(),
                    (_, Tok::End) => return err(ErrorKind::UnclosedParen, pos),
                    _ => unreachable!("expr stops only at End, RParen or an error"),
                }
            }
            Tok::End => return err(ErrorKind::UnexpectedEnd, pos),
            Tok::Bad => return err(ErrorKind::UnexpectedChar, pos),
            Tok::Op(_) | Tok::RParen => return err(ErrorKind::UnexpectedToken, pos),
        }
        for &p in negs.iter().rev() {
            self.out.push(Item::Neg(p));
        }
        Ok(())
    }
}

pub fn evaluate(input: &str) -> Result<i64, EvalError> {
    let chars: Vec<char> = input.chars().collect();
    let mut p = Parser { toks: lex(&chars), at: 0, out: Vec::new() };
    p.expr(0)?;
    // At depth 0 the expression must stop at End; a stray ")" is an error.
    if let (pos, Tok::RParen) = p.peek() {
        return err(ErrorKind::UnexpectedToken, pos);
    }
    let mut stack: Vec<i64> = Vec::new();
    for item in &p.out {
        match *item {
            Item::Push(v) => stack.push(v),
            Item::Neg(pos) => {
                let v = stack.pop().unwrap();
                match v.checked_neg() {
                    Some(r) => stack.push(r),
                    None => return err(ErrorKind::Overflow, pos),
                }
            }
            Item::Bin(op, pos) => {
                let b = stack.pop().unwrap();
                let a = stack.pop().unwrap();
                let r = match op {
                    '+' => a.checked_add(b),
                    '-' => a.checked_sub(b),
                    '*' => a.checked_mul(b),
                    '/' | '%' => {
                        if b == 0 {
                            return err(ErrorKind::DivisionByZero, pos);
                        }
                        if op == '/' {
                            a.checked_div(b)
                        } else {
                            a.checked_rem(b)
                        }
                    }
                    _ => unreachable!(),
                };
                match r {
                    Some(v) => stack.push(v),
                    None => return err(ErrorKind::Overflow, pos),
                }
            }
        }
    }
    Ok(stack.pop().unwrap())
}
