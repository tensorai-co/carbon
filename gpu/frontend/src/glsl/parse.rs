//! GLSL's grammar, by recursive descent over the preprocessor's tokens: a translation unit's declarations, blocks and
//! functions; statements; expressions by precedence. A name is a type if it's one of GLSL's or a struct declared
//! before it, which is how a declaration is told from an expression.

use alloc::boxed::Box;
use alloc::collections::BTreeSet;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use super::ast::*;
use super::pp::{Tok, Token};
use super::{Error, Version, error};

/// GLSL's type names this compiler knows (whether each exists in a version is `lower`'s question).
pub fn is_type_name(name: &str) -> bool {
    matches!(
        name,
        "void" | "bool" | "int" | "uint" | "float" | "double"
            | "vec2" | "vec3" | "vec4" | "ivec2" | "ivec3" | "ivec4" | "uvec2" | "uvec3" | "uvec4" | "bvec2" | "bvec3" | "bvec4"
            | "dvec2" | "dvec3" | "dvec4"
            | "mat2" | "mat3" | "mat4" | "mat2x2" | "mat2x3" | "mat2x4" | "mat3x2" | "mat3x3" | "mat3x4" | "mat4x2" | "mat4x3" | "mat4x4"
            | "dmat2" | "dmat3" | "dmat4"
            | "sampler1D" | "sampler2D" | "sampler3D" | "samplerCube" | "sampler2DShadow" | "samplerCubeShadow" | "sampler2DArray"
            | "sampler2DArrayShadow" | "sampler2DRect" | "sampler2DMS" | "samplerBuffer" | "samplerExternalOES"
            | "isampler2D" | "isampler3D" | "isamplerCube" | "isampler2DArray" | "usampler2D" | "usampler3D" | "usamplerCube" | "usampler2DArray"
    )
}

const QUALIFIERS: &[&str] = &[
    "const", "in", "out", "inout", "uniform", "buffer", "shared", "attribute", "varying", "flat", "smooth", "noperspective", "centroid", "invariant",
    "layout", "lowp", "mediump", "highp", "precise", "readonly", "writeonly", "coherent", "volatile", "restrict", "sample", "patch",
];

pub fn parse(tokens: &[Token], version: Version) -> Result<Vec<External>, Error> {
    let mut p = Parser { toks: tokens, at: 0, structs: BTreeSet::new(), version };
    let mut out = Vec::new();
    while !p.done() {
        if p.eat(";") {
            continue;
        }
        out.push(p.external()?);
    }
    Ok(out)
}

struct Parser<'a> {
    toks: &'a [Token],
    at: usize,
    structs: BTreeSet<String>,
    #[allow(dead_code)]
    version: Version,
}

impl Parser<'_> {
    fn done(&self) -> bool {
        self.at >= self.toks.len()
    }

    fn line(&self) -> u32 {
        self.toks.get(self.at).or(self.toks.last()).map_or(1, |t| t.line)
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at).map(|t| &t.tok)
    }

    fn peek_at(&self, n: usize) -> Option<&Tok> {
        self.toks.get(self.at + n).map(|t| &t.tok)
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Some(Tok::Punct(q)) if *q == p)
    }

    fn is_word(&self, w: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(i)) if i == w)
    }

    fn eat(&mut self, p: &str) -> bool {
        if self.is(p) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn eat_word(&mut self, w: &str) -> bool {
        if self.is_word(w) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, p: &str) -> Result<(), Error> {
        if self.eat(p) {
            return Ok(());
        }
        let found = match self.peek() {
            Some(Tok::Ident(i)) => format!("`{i}`"),
            Some(Tok::Punct(q)) => format!("`{q}`"),
            Some(Tok::Int(v, _)) => format!("{v}"),
            Some(Tok::Float(v, _)) => format!("{v}"),
            None => "the end".into(),
        };
        error(self.line(), &format!("`{p}` expected, {found} found"))
    }

    fn ident(&mut self) -> Result<String, Error> {
        match self.peek() {
            Some(Tok::Ident(i)) if !QUALIFIERS.contains(&i.as_str()) && !is_keyword(i) => {
                let i = i.clone();
                self.at += 1;
                Ok(i)
            }
            Some(Tok::Ident(i)) => error(self.line(), &format!("`{i}` is a keyword, where a name is wanted")),
            _ => error(self.line(), "a name expected"),
        }
    }

    fn is_type_at(&self, n: usize) -> bool {
        match self.peek_at(n) {
            Some(Tok::Ident(i)) => is_type_name(i) || self.structs.contains(i) || i == "struct",
            _ => false,
        }
    }

    fn is_qualifier(&self) -> bool {
        matches!(self.peek(), Some(Tok::Ident(i)) if QUALIFIERS.contains(&i.as_str()))
    }

    // ---- Declarations ----

    fn qualifiers(&mut self) -> Result<Qualifiers, Error> {
        let mut q = Qualifiers::none();
        let line = self.line();
        let set = |q: &mut Qualifiers, s: Storage| -> Result<(), Error> {
            if q.storage != Storage::None {
                return error(line, "two storage qualifiers");
            }
            q.storage = s;
            Ok(())
        };
        while let Some(Tok::Ident(w)) = self.peek().cloned() {
            match w.as_str() {
                "const" => set(&mut q, Storage::Const)?,
                "in" => set(&mut q, Storage::In)?,
                "out" => set(&mut q, Storage::Out)?,
                "inout" => set(&mut q, Storage::InOut)?,
                "uniform" => set(&mut q, Storage::Uniform)?,
                "buffer" => set(&mut q, Storage::Buffer)?,
                "shared" => set(&mut q, Storage::Shared)?,
                "attribute" => set(&mut q, Storage::Attribute)?,
                "varying" => set(&mut q, Storage::Varying)?,
                "flat" => q.interp = Some(Interp::Flat),
                "smooth" => q.interp = Some(Interp::Smooth),
                "noperspective" => q.interp = Some(Interp::NoPerspective),
                "centroid" | "sample" => q.centroid = true,
                "invariant" | "precise" => q.invariant = true,
                "readonly" => q.readonly = true,
                "writeonly" => q.writeonly = true,
                "lowp" | "mediump" | "highp" | "coherent" | "volatile" | "restrict" | "patch" => {}
                "layout" => {
                    self.at += 1;
                    self.expect("(")?;
                    loop {
                        let name = match self.peek().cloned() {
                            Some(Tok::Ident(n)) => n,
                            _ => return error(self.line(), "a layout qualifier's name expected"),
                        };
                        self.at += 1;
                        let value = if self.eat("=") { Some(self.assignment()?) } else { None };
                        q.layout.push((name, value));
                        if !self.eat(",") {
                            break;
                        }
                    }
                    self.expect(")")?;
                    continue;
                }
                _ => break,
            }
            self.at += 1;
        }
        Ok(q)
    }

    /// A type: a name or a struct's definition, and array sizes after it.
    fn type_spec(&mut self) -> Result<TypeSpec, Error> {
        let line = self.line();
        let base = if self.eat_word("struct") {
            let name = if let Some(Tok::Ident(_)) = self.peek() { Some(self.ident()?) } else { None };
            if let Some(n) = &name {
                self.structs.insert(n.clone());
            }
            self.expect("{")?;
            let members = self.members()?;
            TypeBase::Struct(StructDef { name, members, line })
        } else {
            match self.peek().cloned() {
                Some(Tok::Ident(n)) if is_type_name(&n) || self.structs.contains(&n) => {
                    self.at += 1;
                    TypeBase::Named(n)
                }
                _ => return error(line, "a type expected"),
            }
        };
        let dims = self.dims()?;
        Ok(TypeSpec { base, dims })
    }

    /// `[n]` after `[m]`, as many as there are.
    fn dims(&mut self) -> Result<Vec<Option<Expr>>, Error> {
        let mut dims = Vec::new();
        while self.eat("[") {
            if self.eat("]") {
                dims.push(None);
            } else {
                dims.push(Some(self.expression()?));
                self.expect("]")?;
            }
        }
        Ok(dims)
    }

    /// A struct's or block's members, up to its `}`.
    fn members(&mut self) -> Result<Vec<Member>, Error> {
        let mut members = Vec::new();
        while !self.eat("}") {
            let line = self.line();
            let quals = self.qualifiers()?;
            let ty = self.type_spec()?;
            let mut names = Vec::new();
            loop {
                let n = self.ident()?;
                let d = self.dims()?;
                names.push((n, d));
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(";")?;
            members.push(Member { quals, ty, names, line });
        }
        Ok(members)
    }

    fn external(&mut self) -> Result<External, Error> {
        let line = self.line();
        if self.eat_word("precision") {
            // Precision is highp everywhere here: the statement only has to be well formed.
            self.qualifiers()?;
            self.type_spec()?;
            self.expect(";")?;
            return Ok(External::Default { quals: Qualifiers::none(), line });
        }
        let quals = self.qualifiers()?;
        // `layout(...) in;` and the like.
        if self.eat(";") {
            return Ok(External::Default { quals, line });
        }
        // A block: a storage qualifier, then a name and `{`.
        if quals.storage != Storage::None && !self.is_type_at(0) && matches!(self.peek_at(1), Some(Tok::Punct("{"))) {
            let name = self.ident()?;
            self.expect("{")?;
            let members = self.members()?;
            let instance = if let Some(Tok::Ident(_)) = self.peek() {
                let n = self.ident()?;
                let d = self.dims()?;
                Some((n, d))
            } else {
                None
            };
            self.expect(";")?;
            return Ok(External::Block(Block { quals, name, members, instance, line }));
        }
        // `invariant gl_Position;`.
        if !self.is_type_at(0) {
            let mut names = Vec::new();
            loop {
                names.push(self.ident()?);
                if !self.eat(",") {
                    break;
                }
            }
            self.expect(";")?;
            return Ok(External::Requalify { quals, names, line });
        }
        let ty = self.type_spec()?;
        if self.eat(";") {
            return Ok(External::Decl { quals, ty, vars: Vec::new(), line });
        }
        let name = self.ident()?;
        if self.eat("(") {
            let params = self.params()?;
            let body = if self.eat(";") { None } else { Some(self.compound()?) };
            return Ok(External::Function(Function { ret: ty, name, params, body, line }));
        }
        let vars = self.declarators(name)?;
        Ok(External::Decl { quals, ty, vars, line })
    }

    fn params(&mut self) -> Result<Vec<Param>, Error> {
        let mut params = Vec::new();
        if self.eat(")") {
            return Ok(params);
        }
        // `f(void)`.
        if self.is_word("void") && matches!(self.peek_at(1), Some(Tok::Punct(")"))) {
            self.at += 2;
            return Ok(params);
        }
        loop {
            let quals = self.qualifiers()?;
            let mut ty = self.type_spec()?;
            let name = if let Some(Tok::Ident(_)) = self.peek() { Some(self.ident()?) } else { None };
            ty.dims.extend(self.dims()?);
            params.push(Param { quals, ty, name });
            if self.eat(")") {
                break;
            }
            self.expect(",")?;
        }
        Ok(params)
    }

    /// The rest of a declaration after its first name: its sizes, initializer, and any more names.
    fn declarators(&mut self, first: String) -> Result<Vec<Declarator>, Error> {
        let mut vars = Vec::new();
        let mut name = first;
        loop {
            let line = self.line();
            let dims = self.dims()?;
            let init = if self.eat("=") { Some(self.init()?) } else { None };
            vars.push(Declarator { name, dims, init, line });
            if self.eat(";") {
                return Ok(vars);
            }
            self.expect(",")?;
            name = self.ident()?;
        }
    }

    fn init(&mut self) -> Result<Init, Error> {
        if self.eat("{") {
            let mut items = Vec::new();
            loop {
                if self.eat("}") {
                    break;
                }
                items.push(self.init()?);
                if !self.eat(",") {
                    self.expect("}")?;
                    break;
                }
            }
            return Ok(Init::List(items));
        }
        Ok(Init::Expr(self.assignment()?))
    }

    // ---- Statements ----

    fn compound(&mut self) -> Result<Vec<Stmt>, Error> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.eat("}") {
            if self.done() {
                return error(self.line(), "a `{` that isn't closed");
            }
            body.push(self.statement()?);
        }
        Ok(body)
    }

    /// Whether what's next declares a variable rather than being an expression.
    fn is_declaration(&self) -> bool {
        if self.is_qualifier() || self.is_word("struct") || self.is_word("precision") {
            return true;
        }
        if !self.is_type_at(0) {
            return false;
        }
        // A type and a name; or a type, its sizes, and a name.
        let mut n = 1;
        if matches!(self.peek_at(1), Some(Tok::Punct("["))) {
            let mut depth = 0;
            while let Some(t) = self.peek_at(n) {
                match t {
                    Tok::Punct("[") => depth += 1,
                    Tok::Punct("]") => {
                        depth -= 1;
                        if depth == 0 && !matches!(self.peek_at(n + 1), Some(Tok::Punct("["))) {
                            n += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                n += 1;
            }
        }
        matches!(self.peek_at(n), Some(Tok::Ident(_)))
    }

    fn statement(&mut self) -> Result<Stmt, Error> {
        let line = self.line();
        let kind = if self.is("{") {
            StmtKind::Block(self.compound()?)
        } else if self.eat(";") {
            StmtKind::Empty
        } else if self.eat_word("if") {
            self.expect("(")?;
            let cond = self.expression()?;
            self.expect(")")?;
            let then = Box::new(self.statement()?);
            let otherwise = if self.eat_word("else") { Some(Box::new(self.statement()?)) } else { None };
            StmtKind::If(cond, then, otherwise)
        } else if self.eat_word("while") {
            self.expect("(")?;
            let cond = self.condition()?;
            self.expect(")")?;
            StmtKind::While(cond, Box::new(self.statement()?))
        } else if self.eat_word("do") {
            let body = Box::new(self.statement()?);
            if !self.eat_word("while") {
                return error(self.line(), "`while` expected after a do's body");
            }
            self.expect("(")?;
            let cond = self.expression()?;
            self.expect(")")?;
            self.expect(";")?;
            StmtKind::DoWhile(body, cond)
        } else if self.eat_word("for") {
            self.expect("(")?;
            let init = if self.eat(";") { None } else { Some(Box::new(self.simple()?)) };
            let cond = if self.is(";") { None } else { Some(self.condition()?) };
            self.expect(";")?;
            let step = if self.is(")") { None } else { Some(self.expression()?) };
            self.expect(")")?;
            let body = Box::new(self.statement()?);
            StmtKind::For { init, cond, step, body }
        } else if self.eat_word("switch") {
            self.expect("(")?;
            let e = self.expression()?;
            self.expect(")")?;
            StmtKind::Switch(e, self.compound()?)
        } else if self.eat_word("case") {
            let e = self.expression()?;
            self.expect(":")?;
            StmtKind::Case(e)
        } else if self.eat_word("default") {
            self.expect(":")?;
            StmtKind::Default
        } else if self.eat_word("break") {
            self.expect(";")?;
            StmtKind::Break
        } else if self.eat_word("continue") {
            self.expect(";")?;
            StmtKind::Continue
        } else if self.eat_word("discard") {
            self.expect(";")?;
            StmtKind::Discard
        } else if self.eat_word("return") {
            let e = if self.is(";") { None } else { Some(self.expression()?) };
            self.expect(";")?;
            StmtKind::Return(e)
        } else {
            return self.simple();
        };
        Ok(Stmt { kind, line })
    }

    /// A declaration or an expression, with its `;`.
    fn simple(&mut self) -> Result<Stmt, Error> {
        let line = self.line();
        if self.is_word("precision") {
            self.at += 1;
            self.qualifiers()?;
            self.type_spec()?;
            self.expect(";")?;
            return Ok(Stmt { kind: StmtKind::Empty, line });
        }
        if self.is_declaration() {
            let quals = self.qualifiers()?;
            let ty = self.type_spec()?;
            if self.eat(";") {
                return Ok(Stmt { kind: StmtKind::Decl { quals, ty, vars: Vec::new() }, line });
            }
            let name = self.ident()?;
            let vars = self.declarators(name)?;
            return Ok(Stmt { kind: StmtKind::Decl { quals, ty, vars }, line });
        }
        let e = self.expression()?;
        self.expect(";")?;
        Ok(Stmt { kind: StmtKind::Expr(e), line })
    }

    fn condition(&mut self) -> Result<Cond, Error> {
        if self.is_declaration() {
            self.qualifiers()?;
            let ty = self.type_spec()?;
            let name = self.ident()?;
            self.expect("=")?;
            let init = self.assignment()?;
            return Ok(Cond::Decl { ty, name, init });
        }
        Ok(Cond::Expr(self.expression()?))
    }

    // ---- Expressions ----

    pub fn expression(&mut self) -> Result<Expr, Error> {
        let mut e = self.assignment()?;
        while self.is(",") {
            let line = self.line();
            self.at += 1;
            let r = self.assignment()?;
            e = Expr { kind: ExprKind::Comma(Box::new(e), Box::new(r)), line };
        }
        Ok(e)
    }

    fn assignment(&mut self) -> Result<Expr, Error> {
        let left = self.ternary()?;
        let op = match self.peek() {
            Some(Tok::Punct("=")) => None,
            Some(Tok::Punct(p)) => match *p {
                "*=" => Some(Binary::Mul),
                "/=" => Some(Binary::Div),
                "%=" => Some(Binary::Mod),
                "+=" => Some(Binary::Add),
                "-=" => Some(Binary::Sub),
                "<<=" => Some(Binary::Shl),
                ">>=" => Some(Binary::Shr),
                "&=" => Some(Binary::BitAnd),
                "^=" => Some(Binary::BitXor),
                "|=" => Some(Binary::BitOr),
                _ => return Ok(left),
            },
            _ => return Ok(left),
        };
        let line = self.line();
        self.at += 1;
        let right = self.assignment()?;
        Ok(Expr { kind: ExprKind::Assign(op, Box::new(left), Box::new(right)), line })
    }

    fn ternary(&mut self) -> Result<Expr, Error> {
        let cond = self.binary(0)?;
        if !self.is("?") {
            return Ok(cond);
        }
        let line = self.line();
        self.at += 1;
        let a = self.expression()?;
        self.expect(":")?;
        let b = self.assignment()?;
        Ok(Expr { kind: ExprKind::Ternary(Box::new(cond), Box::new(a), Box::new(b)), line })
    }

    fn binary(&mut self, min: u8) -> Result<Expr, Error> {
        let mut left = self.unary()?;
        loop {
            let Some(Tok::Punct(p)) = self.peek() else { return Ok(left) };
            let (prec, op) = match *p {
                "||" => (1, Binary::Or),
                "^^" => (2, Binary::Xor),
                "&&" => (3, Binary::And),
                "|" => (4, Binary::BitOr),
                "^" => (5, Binary::BitXor),
                "&" => (6, Binary::BitAnd),
                "==" => (7, Binary::Eq),
                "!=" => (7, Binary::Ne),
                "<" => (8, Binary::Lt),
                ">" => (8, Binary::Gt),
                "<=" => (8, Binary::Le),
                ">=" => (8, Binary::Ge),
                "<<" => (9, Binary::Shl),
                ">>" => (9, Binary::Shr),
                "+" => (10, Binary::Add),
                "-" => (10, Binary::Sub),
                "*" => (11, Binary::Mul),
                "/" => (11, Binary::Div),
                "%" => (11, Binary::Mod),
                _ => return Ok(left),
            };
            if prec <= min {
                return Ok(left);
            }
            let line = self.line();
            self.at += 1;
            let right = self.binary(prec)?;
            left = Expr { kind: ExprKind::Binary(op, Box::new(left), Box::new(right)), line };
        }
    }

    fn unary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let op = match self.peek() {
            Some(Tok::Punct("-")) => Unary::Neg,
            Some(Tok::Punct("+")) => Unary::Plus,
            Some(Tok::Punct("!")) => Unary::Not,
            Some(Tok::Punct("~")) => Unary::BitNot,
            Some(Tok::Punct("++")) => Unary::PreInc,
            Some(Tok::Punct("--")) => Unary::PreDec,
            _ => return self.postfix(),
        };
        self.at += 1;
        let e = self.unary()?;
        Ok(Expr { kind: ExprKind::Unary(op, Box::new(e)), line })
    }

    fn postfix(&mut self) -> Result<Expr, Error> {
        let mut e = self.primary()?;
        loop {
            let line = self.line();
            if self.eat("[") {
                let i = self.expression()?;
                self.expect("]")?;
                e = Expr { kind: ExprKind::Index(Box::new(e), Box::new(i)), line };
            } else if self.eat(".") {
                let name = match self.peek().cloned() {
                    Some(Tok::Ident(n)) => n,
                    _ => return error(line, "a member's name expected after `.`"),
                };
                self.at += 1;
                if name == "length" && self.is("(") {
                    self.expect("(")?;
                    self.expect(")")?;
                    e = Expr { kind: ExprKind::Length(Box::new(e)), line };
                } else {
                    e = Expr { kind: ExprKind::Field(Box::new(e), name), line };
                }
            } else if self.eat("++") {
                e = Expr { kind: ExprKind::Unary(Unary::PostInc, Box::new(e)), line };
            } else if self.eat("--") {
                e = Expr { kind: ExprKind::Unary(Unary::PostDec, Box::new(e)), line };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, Error> {
        let line = self.line();
        let kind = match self.peek().cloned() {
            Some(Tok::Int(v, u)) => {
                self.at += 1;
                ExprKind::Int(v, u)
            }
            Some(Tok::Float(v, _)) => {
                self.at += 1;
                ExprKind::Float(v)
            }
            Some(Tok::Punct("(")) => {
                self.at += 1;
                let e = self.expression()?;
                self.expect(")")?;
                return Ok(e);
            }
            Some(Tok::Ident(w)) if w == "true" || w == "false" => {
                self.at += 1;
                ExprKind::Bool(w == "true")
            }
            Some(Tok::Ident(_)) if self.is_type_at(0) => {
                // A constructor: a type (with sizes, for an array's), then its arguments.
                let ty = self.type_spec()?;
                self.expect("(")?;
                let args = self.arguments()?;
                ExprKind::Call(Callee::Type(ty), args)
            }
            Some(Tok::Ident(_)) => {
                let name = self.ident()?;
                if self.eat("(") {
                    let args = self.arguments()?;
                    ExprKind::Call(Callee::Name(name), args)
                } else {
                    ExprKind::Ident(name)
                }
            }
            _ => return error(line, "an expression expected"),
        };
        Ok(Expr { kind, line })
    }

    /// A call's arguments, after its `(`.
    fn arguments(&mut self) -> Result<Vec<Expr>, Error> {
        let mut args = Vec::new();
        if self.eat(")") {
            return Ok(args);
        }
        if self.is_word("void") && matches!(self.peek_at(1), Some(Tok::Punct(")"))) {
            self.at += 2;
            return Ok(args);
        }
        loop {
            args.push(self.assignment()?);
            if self.eat(")") {
                return Ok(args);
            }
            self.expect(",")?;
        }
    }
}

/// Words that are GLSL's own and can't be names.
fn is_keyword(w: &str) -> bool {
    matches!(
        w,
        "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "default" | "break" | "continue" | "return" | "discard" | "struct" | "true"
            | "false" | "precision"
    ) || is_type_name(w)
}
