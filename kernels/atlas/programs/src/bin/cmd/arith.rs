//! SET /A's arithmetic: 32-bit integers, cmd's operators by its precedence, variables by name (a name that isn't set
//! is 0), and assignment (`=`, `+=` and the rest) that sets them. Expressions separated by `,` run in turn; the value
//! is the last one's.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use programs::{get_env, set_env};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(i32),
    Name(String),
    Op(String),
    Open,
    Close,
    Comma,
}

fn tokens(text: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            let n = if let Some(h) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
                u32::from_str_radix(h, 16).map(|n| n as i32)
            } else if s.len() > 1 && s.starts_with('0') {
                u32::from_str_radix(&s[1..], 8).map(|n| n as i32)
            } else {
                s.parse::<i64>().map(|n| n as i32).map_err(|_| u32::from_str_radix("x", 10).unwrap_err())
            };
            out.push(Tok::Num(n.map_err(|_| String::from("Invalid number.  Numeric constants are either decimal (17),\r\nhexadecimal (0x11), or octal (021)."))?));
        } else if c.is_alphabetic() || c == '_' || c == '$' || c == '#' || c == '@' || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '$' | '#' | '@' | '.' | '[' | ']')) {
                i += 1;
            }
            out.push(Tok::Name(chars[start..i].iter().collect()));
        } else if c == '(' {
            out.push(Tok::Open);
            i += 1;
        } else if c == ')' {
            out.push(Tok::Close);
            i += 1;
        } else if c == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else {
            let three: String = chars[i..chars.len().min(i + 3)].iter().collect();
            let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
            let op = ["<<=", ">>="].iter().find(|o| three == **o).map(|o| o.to_string()).or_else(|| ["<<", ">>", "+=", "-=", "*=", "/=", "%=", "&=", "^=", "|="].iter().find(|o| two == **o).map(|o| o.to_string()));
            let op = op.unwrap_or_else(|| c.to_string());
            if !"+-*/%!~&|^<>=".contains(op.chars().next().unwrap_or(' ')) {
                return Err(String::from("Missing operator."));
            }
            i += op.chars().count();
            out.push(Tok::Op(op));
        }
    }
    Ok(out)
}

struct P {
    toks: Vec<Tok>,
    pos: usize,
}

fn value_of(name: &str) -> i32 {
    get_env(name).and_then(|v| v.trim().parse::<i64>().ok()).map_or(0, |n| n as i32)
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn op(&self) -> Option<&str> {
        match self.peek() {
            Some(Tok::Op(o)) => Some(o.as_str()),
            _ => None,
        }
    }

    fn comma(&mut self) -> Result<i32, String> {
        let mut v = self.assign()?;
        while self.peek() == Some(&Tok::Comma) {
            self.pos += 1;
            v = self.assign()?;
        }
        Ok(v)
    }

    fn assign(&mut self) -> Result<i32, String> {
        if let (Some(Tok::Name(name)), Some(Tok::Op(op))) = (self.toks.get(self.pos).cloned(), self.toks.get(self.pos + 1).cloned())
            && op.ends_with('=')
            && op != "=="
        {
            self.pos += 2;
            let right = self.assign()?;
            let left = value_of(&name);
            let v = match op.as_str() {
                "=" => right,
                "+=" => left.wrapping_add(right),
                "-=" => left.wrapping_sub(right),
                "*=" => left.wrapping_mul(right),
                "/=" => div(left, right)?,
                "%=" => rem(left, right)?,
                "&=" => left & right,
                "|=" => left | right,
                "^=" => left ^ right,
                "<<=" => left.wrapping_shl(right as u32),
                ">>=" => left.wrapping_shr(right as u32),
                _ => return Err(String::from("Missing operator.")),
            };
            set_env(&name, Some(&format!("{v}")));
            return Ok(v);
        }
        self.binary(0)
    }

    fn binary(&mut self, level: usize) -> Result<i32, String> {
        const LEVELS: [&[&str]; 6] = [&["|"], &["^"], &["&"], &["<<", ">>"], &["+", "-"], &["*", "/", "%"]];
        if level == LEVELS.len() {
            return self.unary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.op().filter(|o| LEVELS[level].contains(o)).map(String::from) {
            self.pos += 1;
            let right = self.binary(level + 1)?;
            left = match op.as_str() {
                "|" => left | right,
                "^" => left ^ right,
                "&" => left & right,
                "<<" => left.wrapping_shl(right as u32),
                ">>" => left.wrapping_shr(right as u32),
                "+" => left.wrapping_add(right),
                "-" => left.wrapping_sub(right),
                "*" => left.wrapping_mul(right),
                "/" => div(left, right)?,
                _ => rem(left, right)?,
            };
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<i32, String> {
        match self.op().map(String::from).as_deref() {
            Some("-") => {
                self.pos += 1;
                Ok(self.unary()?.wrapping_neg())
            }
            Some("+") => {
                self.pos += 1;
                self.unary()
            }
            Some("!") => {
                self.pos += 1;
                Ok(i32::from(self.unary()? == 0))
            }
            Some("~") => {
                self.pos += 1;
                Ok(!self.unary()?)
            }
            _ => self.atom(),
        }
    }

    fn atom(&mut self) -> Result<i32, String> {
        match self.toks.get(self.pos).cloned() {
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(n)
            }
            Some(Tok::Name(name)) => {
                self.pos += 1;
                Ok(value_of(&name))
            }
            Some(Tok::Open) => {
                self.pos += 1;
                let v = self.comma()?;
                if self.peek() != Some(&Tok::Close) {
                    return Err(String::from("Missing ')'"));
                }
                self.pos += 1;
                Ok(v)
            }
            _ => Err(String::from("Missing operand.")),
        }
    }
}

fn div(a: i32, b: i32) -> Result<i32, String> {
    if b == 0 { Err(String::from("Divide by zero error.")) } else { Ok(a.wrapping_div(b)) }
}

fn rem(a: i32, b: i32) -> Result<i32, String> {
    if b == 0 { Err(String::from("Divide by zero error.")) } else { Ok(a.wrapping_rem(b)) }
}

pub fn eval(text: &str) -> Result<i32, String> {
    let toks = tokens(text)?;
    if toks.is_empty() {
        return Err(String::from("The syntax of the command is incorrect."));
    }
    let mut p = P { toks, pos: 0 };
    let v = p.comma()?;
    if p.pos < p.toks.len() {
        return Err(String::from("Missing operator."));
    }
    Ok(v)
}
