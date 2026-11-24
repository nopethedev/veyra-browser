//! Recursive-descent / Pratt parser for JavaScript.

use std::rc::Rc;

use crate::ast::*;
use crate::lexer::{tokenize, Tok, Token};

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub line: u32,
    pub offset: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

pub fn parse_program(src: &str) -> Result<Vec<Stmt>, ParseError> {
    Ok(parse_module(src)?.body)
}

/// Parses an ES module, keeping import/export metadata separate from the
/// executable statements. Classic scripts parse through the same entry point;
/// a classic script with imports is still rejected by the HTML loader before
/// evaluation when it is not a module.
pub fn parse_module(src: &str) -> Result<ModuleAst, ParseError> {
    let tokens = tokenize(src).map_err(|e| ParseError { message: e.message, line: e.line, offset: 0 })?;
    let mut p = Parser { toks: tokens, pos: 0, no_in: false };
    let mut module = ModuleAst::default();
    while !p.at_eof() {
        p.parse_module_item(&mut module)?;
    }
    Ok(module)
}

fn pattern_names(pat: &Pattern) -> Vec<String> {
    match pat {
        Pattern::Ident(name) => vec![name.clone()],
        Pattern::Array { items, rest } => {
            let mut out = Vec::new();
            for item in items.iter().flatten() {
                out.extend(pattern_names(item));
            }
            if let Some(rest) = rest {
                out.extend(pattern_names(rest));
            }
            out
        }
        Pattern::Object { props, rest } => {
            let mut out = Vec::new();
            for (_, value, _) in props {
                if let Some(value) = value {
                    out.extend(pattern_names(value));
                }
            }
            if let Some(rest) = rest {
                out.extend(pattern_names(rest));
            }
            out
        }
        Pattern::Default(inner, _) | Pattern::Rest(inner) => pattern_names(inner),
    }
}

pub fn parse_expression(src: &str) -> Result<Expr, ParseError> {
    let tokens = tokenize(src).map_err(|e| ParseError { message: e.message, line: e.line, offset: 0 })?;
    let mut p = Parser { toks: tokens, pos: 0, no_in: false };
    let e = p.parse_expr()?;
    if !p.at_eof() {
        return p.err(format!("unexpected {} after expression", p.peek().describe()));
    }
    Ok(e)
}

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    no_in: bool,
}

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks.get(self.pos).unwrap_or(&self.toks[self.toks.len() - 1]).kind
    }
    fn peek_at(&self, n: usize) -> &Tok {
        &self.toks.get(self.pos + n).unwrap_or(&self.toks[self.toks.len() - 1]).kind
    }
    fn line(&self) -> u32 {
        self.toks.get(self.pos).map(|t| t.line).unwrap_or(1)
    }
    fn newline_before(&self) -> bool {
        self.toks.get(self.pos).map(|t| t.newline_before).unwrap_or(false)
    }
    fn at_eof(&self) -> bool {
        matches!(self.peek(), Tok::Eof)
    }
    fn bump(&mut self) -> Tok {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            message: msg.into(),
            line: self.line(),
            offset: self.toks.get(self.pos).map(|t| t.offset).unwrap_or(0),
        })
    }
    fn eat_punct(&mut self, p: &str) -> bool {
        if self.peek().is_punct(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_punct(&mut self, p: &str) -> Result<(), ParseError> {
        if self.eat_punct(p) {
            Ok(())
        } else {
            self.err(format!("expected `{p}`, found {}", self.peek().describe()))
        }
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.peek().is_keyword(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn peek_is_ident(&self, name: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == name)
    }
    fn eat_ident(&mut self, name: &str) -> bool {
        if self.peek_is_ident(name) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_as(&mut self) -> Result<(), ParseError> {
        if self.eat_kw("as") || self.eat_ident("as") {
            Ok(())
        } else {
            self.err(format!("expected `as`, found {}", self.peek().describe()))
        }
    }
    fn expect_ident(&mut self, name: &str) -> Result<(), ParseError> {
        if self.eat_ident(name) {
            Ok(())
        } else {
            self.err(format!("expected `{name}`, found {}", self.peek().describe()))
        }
    }
    fn expect_kw(&mut self, k: &str) -> Result<(), ParseError> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected `{k}`, found {}", self.peek().describe()))
        }
    }
    fn ident_name(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            // Keywords are valid property names but not bindings; callers that
            // need property names use `parse_prop_key`.
            other => self.err(format!("expected a name, found {}", other.describe())),
        }
    }
    fn semicolon(&mut self) {
        self.eat_punct(";");
    }

    // ------------------------------------------------------------------
    // Modules
    // ------------------------------------------------------------------

    fn parse_module_item(&mut self, module: &mut ModuleAst) -> Result<(), ParseError> {
        if self.peek().is_keyword("import") && !self.peek_at(1).is_punct("(") {
            self.bump();
            let mut binding = ImportBinding {
                source: String::new(),
                default: None,
                namespace: None,
                named: Vec::new(),
            };
            if let Tok::Str(s) = self.peek().clone() {
                self.bump();
                binding.source = s;
                self.semicolon();
                module.imports.push(binding);
                return Ok(());
            }
            if self.peek().is_punct("{") {
                self.parse_import_named(&mut binding)?;
            } else if self.eat_punct("*") {
                self.expect_as()?;
                binding.namespace = Some(self.ident_name()?);
            } else {
                binding.default = Some(self.ident_name()?);
                if self.eat_punct(",") {
                    if self.peek().is_punct("{") {
                        self.parse_import_named(&mut binding)?;
                    } else {
                        self.expect_punct("*")?;
                        self.expect_as()?;
                        binding.namespace = Some(self.ident_name()?);
                    }
                }
            }
            self.expect_ident("from")?;
            binding.source = match self.peek().clone() {
                Tok::Str(s) => {
                    self.bump();
                    s
                }
                _ => return self.err("expected a module specifier"),
            };
            self.semicolon();
            module.imports.push(binding);
            return Ok(());
        }
        if self.peek().is_keyword("export") {
            self.bump();
            if self.eat_kw("default") {
                if self.peek().is_keyword("function") {
                    let mut stmt = self.parse_function_decl(false)?;
                    if let Stmt::FuncDecl(f) = &mut stmt {
                        let mut node = (**f).clone();
                        if node.name.is_none() {
                            node.name = Some("__default__".into());
                            node.self_bind = true;
                        }
                        let name = node.name.clone().unwrap();
                        module.exports.push(("default".into(), name));
                        module.body.push(Stmt::FuncDecl(Rc::new(node)));
                        return Ok(());
                    }
                    module.body.push(stmt);
                    return Ok(());
                }
                if self.eat_kw("async") {
                    let mut node = match self.parse_function_expr(true)? {
                        Expr::Function(f) => f,
                        _ => return self.err("invalid default export"),
                    };
                    let mut n = (*node).clone();
                    if n.name.is_none() {
                        n.name = Some("__default__".into());
                        n.self_bind = true;
                    }
                    let name = n.name.clone().unwrap();
                    node = Rc::new(n);
                    module.exports.push(("default".into(), name));
                    module.body.push(Stmt::FuncDecl(node));
                    return Ok(());
                }
                if self.peek().is_keyword("class") {
                    let stmt = self.parse_class_decl()?;
                    if let Stmt::ClassDecl(c) = &stmt {
                        let mut node = (**c).clone();
                        if node.name.is_none() {
                            node.name = Some("__default__".into());
                        }
                        let name = node.name.clone().unwrap();
                        module.exports.push(("default".into(), name));
                        module.body.push(Stmt::ClassDecl(Rc::new(node)));
                    } else {
                        module.body.push(stmt);
                    }
                    return Ok(());
                }
                let expr = self.parse_assign()?;
                self.semicolon();
                module.exports.push(("default".into(), "__default__".into()));
                module.body.push(Stmt::Var {
                    kind: VarKind::Const,
                    decls: vec![(Pattern::Ident("__default__".into()), Some(expr))],
                });
                return Ok(());
            }
            if self.eat_punct("*") {
                let alias = if (self.eat_kw("as") || self.eat_ident("as")) {
                    Some(self.parse_member_name()?)
                } else {
                    None
                };
                self.expect_ident("from")?;
                let source = match self.peek().clone() {
                    Tok::Str(s) => {
                        self.bump();
                        s
                    }
                    _ => return self.err("expected a module specifier"),
                };
                self.semicolon();
                if let Some(alias) = alias {
                    module.reexports.push((source, "*".into(), alias));
                } else {
                    module.export_all.push(source);
                }
                return Ok(());
            }
            if self.peek().is_punct("{") {
                let (items, from) = self.parse_export_list()?;
                for (local, exported) in items {
                    match &from {
                        Some(source) => module.reexports.push((source.clone(), local, exported)),
                        None => module.exports.push((exported, local)),
                    }
                }
                return Ok(());
            }
            if self.peek().is_keyword("function") {
                let stmt = self.parse_function_decl(false)?;
                if let Stmt::FuncDecl(f) = &stmt {
                    if let Some(name) = &f.name {
                        module.exports.push((name.clone(), name.clone()));
                    }
                }
                module.body.push(stmt);
                return Ok(());
            }
            if self.peek().is_keyword("async") && self.peek_at(1).is_keyword("function") {
                self.bump();
                let stmt = self.parse_function_decl(true)?;
                if let Stmt::FuncDecl(f) = &stmt {
                    if let Some(name) = &f.name {
                        module.exports.push((name.clone(), name.clone()));
                    }
                }
                module.body.push(stmt);
                return Ok(());
            }
            if self.peek().is_keyword("class") {
                let stmt = self.parse_class_decl()?;
                if let Stmt::ClassDecl(c) = &stmt {
                    if let Some(name) = &c.name {
                        module.exports.push((name.clone(), name.clone()));
                    }
                }
                module.body.push(stmt);
                return Ok(());
            }
            if matches!(self.peek(), Tok::Keyword(k) if k == "var" || k == "let" || k == "const") {
                let stmt = self.parse_var_stmt(true)?;
                if let Stmt::Var { decls, .. } = &stmt {
                    for (pat, _) in decls {
                        for name in pattern_names(pat) {
                            module.exports.push((name.clone(), name));
                        }
                    }
                }
                module.body.push(stmt);
                return Ok(());
            }
            return self.err("unsupported export declaration");
        }
        module.body.push(self.parse_stmt()?);
        Ok(())
    }

    fn parse_import_named(&mut self, binding: &mut ImportBinding) -> Result<(), ParseError> {
        self.expect_punct("{")?;
        while !self.peek().is_punct("}") {
            let imported = self.parse_member_name()?;
            let local = if (self.eat_kw("as") || self.eat_ident("as")) {
                self.parse_member_name()?
            } else {
                imported.clone()
            };
            binding.named.push((imported, local));
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}")?;
        Ok(())
    }

    fn parse_export_list(&mut self) -> Result<(Vec<(String, String)>, Option<String>), ParseError> {
        self.expect_punct("{")?;
        let mut items = Vec::new();
        while !self.peek().is_punct("}") {
            let local = self.parse_member_name()?;
            let exported = if (self.eat_kw("as") || self.eat_ident("as")) {
                self.parse_member_name()?
            } else {
                local.clone()
            };
            items.push((local, exported));
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}")?;
        let from = if (self.eat_kw("from") || self.eat_ident("from")) {
            match self.peek().clone() {
                Tok::Str(s) => {
                    self.bump();
                    Some(s)
                }
                _ => return self.err("expected a module specifier"),
            }
        } else {
            None
        };
        self.semicolon();
        Ok((items, from))
    }

    // ------------------------------------------------------------------
    // Statements
    // ------------------------------------------------------------------

    fn parse_stmt(&mut self) -> Result<Stmt, ParseError> {
        match self.peek().clone() {
            Tok::Punct(p) if p == "{" => {
                self.bump();
                let body = self.parse_block_body()?;
                Ok(Stmt::Block(body))
            }
            Tok::Punct(p) if p == ";" => {
                self.bump();
                Ok(Stmt::Empty)
            }
            Tok::Keyword(k) => match k.as_str() {
                "var" | "let" | "const" => self.parse_var_stmt(true),
                "function" => self.parse_function_decl(false),
                "class" => self.parse_class_decl(),
                "if" => self.parse_if(),
                "for" => self.parse_for(),
                "while" => {
                    self.bump();
                    self.expect_punct("(")?;
                    let test = self.parse_expr()?;
                    self.expect_punct(")")?;
                    let body = Box::new(self.parse_stmt()?);
                    Ok(Stmt::While { test, body })
                }
                "do" => {
                    self.bump();
                    let body = Box::new(self.parse_stmt()?);
                    self.expect_kw("while")?;
                    self.expect_punct("(")?;
                    let test = self.parse_expr()?;
                    self.expect_punct(")")?;
                    self.semicolon();
                    Ok(Stmt::DoWhile { body, test })
                }
                "return" => {
                    self.bump();
                    if self.peek().is_punct(";") || self.peek().is_punct("}") || self.at_eof() || self.newline_before() {
                        self.semicolon();
                        Ok(Stmt::Return(None))
                    } else {
                        let e = self.parse_expr()?;
                        self.semicolon();
                        Ok(Stmt::Return(Some(e)))
                    }
                }
                "throw" => {
                    self.bump();
                    let e = self.parse_expr()?;
                    self.semicolon();
                    Ok(Stmt::Throw(e))
                }
                "break" => {
                    self.bump();
                    let label = if let Tok::Ident(name) = self.peek().clone() {
                        self.bump();
                        Some(name)
                    } else {
                        None
                    };
                    self.semicolon();
                    Ok(Stmt::Break(label))
                }
                "continue" => {
                    self.bump();
                    let label = if let Tok::Ident(name) = self.peek().clone() {
                        self.bump();
                        Some(name)
                    } else {
                        None
                    };
                    self.semicolon();
                    Ok(Stmt::Continue(label))
                }
                "try" => self.parse_try(),
                "switch" => self.parse_switch(),
                "debugger" => {
                    self.bump();
                    self.semicolon();
                    Ok(Stmt::Debugger)
                }
                "import" if self.peek_at(1).is_punct("(") || self.peek_at(1).is_punct(".") => {
                    let e = self.parse_expr()?;
                    self.semicolon();
                    Ok(Stmt::Expr(e))
                }
                "import" => self.err("unexpected `import` inside a block"),
                "export" => self.err("unexpected `export` inside a block"),
                "with" => self.err("`with` statements are not supported"),
                _ => {
                    let e = self.parse_expr()?;
                    self.semicolon();
                    Ok(Stmt::Expr(e))
                }
            },
            Tok::Ident(name) if name == "async" && self.peek_at(1).is_keyword("function") => {
                self.bump();
                self.parse_function_decl(true)
            }
            Tok::Ident(name) if self.peek_at(1).is_punct(":") => {
                self.bump();
                self.bump();
                let body = Box::new(self.parse_stmt()?);
                Ok(Stmt::Labeled { label: name, body })
            }
            _ => {
                let e = self.parse_expr()?;
                self.semicolon();
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn parse_block_body(&mut self) -> Result<Vec<Stmt>, ParseError> {
        let mut body = Vec::new();
        while !self.peek().is_punct("}") {
            if self.at_eof() {
                return self.err("unterminated block");
            }
            let line = self.line();
            let stmt = self.parse_stmt()?;
            body.push(Stmt::Located { line, stmt: Box::new(stmt) });
        }
        self.expect_punct("}")?;
        Ok(body)
    }

    fn parse_var_stmt(&mut self, semi: bool) -> Result<Stmt, ParseError> {
        let kind = self.var_kind()?;
        let decls = self.parse_var_decls()?;
        if semi {
            self.semicolon();
        }
        Ok(Stmt::Var { kind, decls })
    }

    fn var_kind(&mut self) -> Result<VarKind, ParseError> {
        if self.eat_kw("var") {
            Ok(VarKind::Var)
        } else if self.eat_kw("let") {
            Ok(VarKind::Let)
        } else if self.eat_kw("const") {
            Ok(VarKind::Const)
        } else {
            self.err("expected a declaration keyword")
        }
    }

    fn parse_var_decls(&mut self) -> Result<Vec<(Pattern, Option<Expr>)>, ParseError> {
        let mut decls = Vec::new();
        loop {
            let pat = self.parse_pattern_top()?;
            let init = if self.eat_punct("=") {
                Some(self.parse_assign()?)
            } else {
                None
            };
            decls.push((pat, init));
            if !self.eat_punct(",") {
                break;
            }
        }
        Ok(decls)
    }

    fn parse_function_decl(&mut self, is_async: bool) -> Result<Stmt, ParseError> {
        let line = self.line();
        self.expect_kw("function")?;
        let is_generator = self.eat_punct("*");
        let name = if matches!(self.peek(), Tok::Ident(_)) {
            Some(self.ident_name()?)
        } else {
            None
        };
        let (params, body) = self.parse_function_rest()?;
        Ok(Stmt::FuncDecl(Rc::new(FnNode {
            name,
            params,
            body,
            is_arrow: false,
            is_async,
            is_generator,
            self_bind: false,
            line,
        })))
    }

    fn parse_function_rest(&mut self) -> Result<(Vec<Pattern>, FnBody), ParseError> {
        self.expect_punct("(")?;
        let mut params = Vec::new();
        while !self.peek().is_punct(")") {
            if self.eat_punct("...") {
                params.push(Pattern::Rest(Box::new(self.parse_pattern()?)));
                break;
            }
            params.push(self.parse_pattern()?);
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")")?;
        let body = self.parse_function_body()?;
        Ok((params, body))
    }

    fn parse_function_body(&mut self) -> Result<FnBody, ParseError> {
        self.expect_punct("{")?;
        Ok(FnBody::Block(self.parse_block_body()?))
    }

    fn parse_class_decl(&mut self) -> Result<Stmt, ParseError> {
        self.expect_kw("class")?;
        let name = if matches!(self.peek(), Tok::Ident(_)) {
            Some(self.ident_name()?)
        } else {
            None
        };
        let node = self.parse_class_tail(name)?;
        Ok(Stmt::ClassDecl(Rc::new(node)))
    }

    fn parse_class_tail(&mut self, name: Option<String>) -> Result<ClassNode, ParseError> {
        let super_class = if self.eat_kw("extends") {
            Some(self.parse_unary()?)
        } else {
            None
        };
        self.expect_punct("{")?;
        let mut members = Vec::new();
        while !self.peek().is_punct("}") {
            if self.at_eof() {
                return self.err("unterminated class body");
            }
            if self.eat_punct(";") {
                continue;
            }
            let mut is_static = false;
            if matches!(self.peek(), Tok::Ident(s) if s == "static")
                && !self.peek_at(1).is_punct("(")
                && !self.peek_at(1).is_punct("=")
            {
                self.bump();
                is_static = true;
            }
            if is_static && self.peek().is_punct("{") {
                let line = self.line();
                let body = self.parse_block_body()?;
                let value = Expr::Function(Rc::new(FnNode {
                    name: None,
                    params: Vec::new(),
                    body: FnBody::Block(body),
                    is_arrow: false,
                    is_async: false,
                    is_generator: false,
                    self_bind: false,
                    line,
                }));
                members.push(ClassMember {
                    key: PropKey::Static("__static_block".into()),
                    value: Some(value),
                    kind: MemberKind::StaticBlock,
                    is_static: true,
                });
                continue;
            }
            let mut kind = MemberKind::Method;
            if matches!(self.peek(), Tok::Ident(s) if s == "get" || s == "set")
                && !self.peek_at(1).is_punct("(")
                && !self.peek_at(1).is_punct("=")
            {
                let is_get = matches!(self.peek(), Tok::Ident(s) if s == "get");
                self.bump();
                kind = if is_get { MemberKind::Get } else { MemberKind::Set };
            }
            let mut is_async = false;
            if matches!(self.peek(), Tok::Ident(s) if s == "async")
                && !self.peek_at(1).is_punct("(")
                && !self.peek_at(1).is_punct("=")
            {
                self.bump();
                is_async = true;
            }
            let is_generator = self.eat_punct("*");
            let key = self.parse_prop_key()?;
            if self.peek().is_punct("(") {
                let (params, body) = self.parse_function_rest()?;
                let line = self.line();
                let name = match &key {
                    PropKey::Static(s) => Some(match kind {
                        MemberKind::Get => format!("get {s}"),
                        MemberKind::Set => format!("set {s}"),
                        _ => s.clone(),
                    }),
                    _ => None,
                };
                let value = Expr::Function(Rc::new(FnNode {
                    name,
                    params,
                    body,
                    is_arrow: false,
                    is_async,
                    is_generator,
                    self_bind: false,
                    line,
                }));
                members.push(ClassMember { key, value: Some(value), kind, is_static });
            } else {
                let value = if self.eat_punct("=") {
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                members.push(ClassMember { key, value, kind: MemberKind::Field, is_static });
                self.semicolon();
            }
        }
        self.expect_punct("}")?;
        Ok(ClassNode { name, super_class, members })
    }

    fn parse_if(&mut self) -> Result<Stmt, ParseError> {
        self.expect_kw("if")?;
        self.expect_punct("(")?;
        let test = self.parse_expr()?;
        self.expect_punct(")")?;
        let cons = Box::new(self.parse_stmt()?);
        let alt = if self.eat_kw("else") {
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If { test, cons, alt })
    }

    fn parse_for(&mut self) -> Result<Stmt, ParseError> {
        self.expect_kw("for")?;
        // `for await (...)` iterates asynchronously. Values in Veyra settle
        // synchronously, so the loop can be treated as a plain for-of.
        self.eat_kw("await");
        self.expect_punct("(")?;
        if self.eat_punct(";") {
            let test = if !self.peek().is_punct(";") { Some(self.parse_expr()?) } else { None };
            self.expect_punct(";")?;
            let update = if !self.peek().is_punct(")") { Some(self.parse_expr()?) } else { None };
            self.expect_punct(")")?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(Stmt::For { init: None, test, update, body });
        }

        let mut init_stmt: Option<Box<Stmt>> = None;
        let mut left: Option<ForLeft> = None;
        if matches!(self.peek(), Tok::Keyword(k) if k == "var" || k == "let" || k == "const") {
            let kind = self.var_kind()?;
            let pat = self.parse_pattern_top()?;
            if self.peek().is_keyword("in") || self.peek_is_ident("of") {
                left = Some(ForLeft::Var(kind, pat));
            } else {
                let init = if self.eat_punct("=") { Some(self.parse_assign()?) } else { None };
                let mut decls = vec![(pat, init)];
                while self.eat_punct(",") {
                    let p = self.parse_pattern_top()?;
                    let i = if self.eat_punct("=") { Some(self.parse_assign()?) } else { None };
                    decls.push((p, i));
                }
                init_stmt = Some(Box::new(Stmt::Var { kind, decls }));
            }
        } else if !self.peek().is_punct(";") {
            self.no_in = true;
            let expr = self.parse_expr();
            self.no_in = false;
            let expr = expr?;
            if self.peek().is_keyword("in") || self.peek_is_ident("of") {
                left = Some(ForLeft::Pat(expr_to_pattern(expr)));
            } else {
                init_stmt = Some(Box::new(Stmt::Expr(expr)));
            }
        }

        if let Some(left) = left {
            let of = self.eat_ident("of");
            if !of {
                self.expect_kw("in")?;
            }
            let right = self.parse_expr()?;
            self.expect_punct(")")?;
            let body = Box::new(self.parse_stmt()?);
            return Ok(Stmt::ForIn { left, right, body, of });
        }

        self.expect_punct(";")?;
        let test = if !self.peek().is_punct(";") { Some(self.parse_expr()?) } else { None };
        self.expect_punct(";")?;
        let update = if !self.peek().is_punct(")") { Some(self.parse_expr()?) } else { None };
        self.expect_punct(")")?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::For { init: init_stmt, test, update, body })
    }

    fn parse_try(&mut self) -> Result<Stmt, ParseError> {
        self.expect_kw("try")?;
        self.expect_punct("{")?;
        let body = Box::new(Stmt::Block(self.parse_block_body()?));
        let mut catch = None;
        let mut finally = None;
        if self.eat_kw("catch") {
            let param = if self.eat_punct("(") {
                let p = self.parse_pattern()?;
                self.expect_punct(")")?;
                Some(p)
            } else {
                None
            };
            self.expect_punct("{")?;
            let b = Box::new(Stmt::Block(self.parse_block_body()?));
            catch = Some((param, b));
        }
        if self.eat_kw("finally") {
            self.expect_punct("{")?;
            finally = Some(Box::new(Stmt::Block(self.parse_block_body()?)));
        }
        if catch.is_none() && finally.is_none() {
            return self.err("`try` requires `catch` or `finally`");
        }
        Ok(Stmt::Try { body, catch, finally })
    }

    fn parse_switch(&mut self) -> Result<Stmt, ParseError> {
        self.expect_kw("switch")?;
        self.expect_punct("(")?;
        let disc = self.parse_expr()?;
        self.expect_punct(")")?;
        self.expect_punct("{")?;
        let mut cases = Vec::new();
        while !self.peek().is_punct("}") {
            let test = if self.eat_kw("case") {
                Some(self.parse_expr()?)
            } else {
                self.expect_kw("default")?;
                None
            };
            self.expect_punct(":")?;
            let mut stmts = Vec::new();
            while !self.peek().is_punct("}") && !self.peek().is_keyword("case") && !self.peek().is_keyword("default") {
                stmts.push(self.parse_stmt()?);
            }
            cases.push((test, stmts));
        }
        self.expect_punct("}")?;
        Ok(Stmt::Switch { disc, cases })
    }

    // ------------------------------------------------------------------
    // Patterns
    // ------------------------------------------------------------------

    fn parse_pattern(&mut self) -> Result<Pattern, ParseError> {
        self.parse_pattern_opt(true)
    }

    /// A binding pattern at the head of a declaration must not swallow the
    /// initializer (`var x = 1`) as a pattern default.
    fn parse_pattern_top(&mut self) -> Result<Pattern, ParseError> {
        self.parse_pattern_opt(false)
    }

    fn parse_pattern_opt(&mut self, allow_default: bool) -> Result<Pattern, ParseError> {
        let mut pat = match self.peek().clone() {
            Tok::Ident(name) => {
                self.bump();
                Pattern::Ident(name)
            }
            Tok::Punct(p) if p == "[" => {
                self.bump();
                let mut items = Vec::new();
                let mut rest = None;
                while !self.peek().is_punct("]") {
                    if self.eat_punct(",") {
                        items.push(None);
                        continue;
                    }
                    if self.eat_punct("...") {
                        rest = Some(Box::new(self.parse_pattern()?));
                        break;
                    }
                    items.push(Some(self.parse_pattern()?));
                    if !self.eat_punct(",") {
                        break;
                    }
                }
                self.expect_punct("]")?;
                Pattern::Array { items, rest }
            }
            Tok::Punct(p) if p == "{" => {
                self.bump();
                let mut props = Vec::new();
                let mut rest = None;
                while !self.peek().is_punct("}") {
                    if self.eat_punct("...") {
                        rest = Some(Box::new(self.parse_pattern()?));
                        break;
                    }
                    let key = self.parse_prop_key()?;
                    let mut pat = if self.eat_punct(":") {
                        Some(self.parse_pattern()?)
                    } else {
                        match &key {
                            PropKey::Static(name) => Some(Pattern::Ident(name.clone())),
                            PropKey::Computed(_) => None,
                        }
                    };
                    let mut default = None;
                    if self.eat_punct("=") {
                        default = Some(self.parse_assign()?);
                    } else if pat.is_none() {
                        return self.err("computed object patterns need a binding");
                    }
                    if let (Some(Pattern::Default(_, _)), Some(_)) = (&pat, &default) {
                        // Already handled by parse_pattern.
                    }
                    if let Some(d) = default {
                        pat = Some(Pattern::Default(Box::new(pat.unwrap()), Box::new(d)));
                    }
                    props.push((key, pat, None));
                    if !self.eat_punct(",") {
                        break;
                    }
                }
                self.expect_punct("}")?;
                Pattern::Object { props, rest }
            }
            other => return self.err(format!("expected a binding pattern, found {}", other.describe())),
        };
        if allow_default && self.eat_punct("=") {
            let default = self.parse_assign()?;
            pat = Pattern::Default(Box::new(pat), Box::new(default));
        }
        Ok(pat)
    }

    fn parse_prop_key(&mut self) -> Result<PropKey, ParseError> {
        match self.peek().clone() {
            Tok::Ident(name) => {
                self.bump();
                Ok(PropKey::Static(name))
            }
            Tok::Keyword(k) => {
                self.bump();
                Ok(PropKey::Static(k))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(PropKey::Static(s))
            }
            Tok::Num(n) => {
                self.bump();
                Ok(PropKey::Static(number_key(n)))
            }
            Tok::BigInt(n) => {
                self.bump();
                Ok(PropKey::Static(n.to_string()))
            }
            Tok::Punct(p) if p == "[" => {
                self.bump();
                let e = self.parse_assign()?;
                self.expect_punct("]")?;
                Ok(PropKey::Computed(e))
            }
            other => self.err(format!("expected a property name, found {}", other.describe())),
        }
    }

    // ------------------------------------------------------------------
    // Expressions
    // ------------------------------------------------------------------

    pub fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        let first = self.parse_assign()?;
        if self.eat_punct(",") {
            let mut items = vec![first];
            loop {
                items.push(self.parse_assign()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
            Ok(Expr::Seq(items))
        } else {
            Ok(first)
        }
    }

    fn parse_assign(&mut self) -> Result<Expr, ParseError> {
        if self.peek().is_keyword("yield") {
            self.bump();
            let delegate = self.eat_punct("*");
            let ends = self.at_eof()
                || self.peek().is_punct(";")
                || self.peek().is_punct(")")
                || self.peek().is_punct("]")
                || self.peek().is_punct("}")
                || self.peek().is_punct(",")
                || self.newline_before();
            let value = if ends { None } else { Some(Box::new(self.parse_assign()?)) };
            return Ok(Expr::Yield { value, delegate });
        }
        if self.arrow_ahead() {
            return self.parse_arrow();
        }
        let left = self.parse_conditional()?;
        let op = match self.peek() {
            Tok::Punct(p) => match p.as_str() {
                "=" => Some(AssignOp::Assign),
                "+=" => Some(AssignOp::Add),
                "-=" => Some(AssignOp::Sub),
                "*=" => Some(AssignOp::Mul),
                "/=" => Some(AssignOp::Div),
                "%=" => Some(AssignOp::Mod),
                "**=" => Some(AssignOp::Exp),
                "&=" => Some(AssignOp::BitAnd),
                "|=" => Some(AssignOp::BitOr),
                "^=" => Some(AssignOp::BitXor),
                "<<=" => Some(AssignOp::Shl),
                ">>=" => Some(AssignOp::Shr),
                ">>>=" => Some(AssignOp::UShr),
                "&&=" => Some(AssignOp::And),
                "||=" => Some(AssignOp::Or),
                "??=" => Some(AssignOp::Nullish),
                _ => None,
            },
            _ => None,
        };
        if let Some(op) = op {
            self.bump();
            let right = self.parse_assign()?;
            return Ok(Expr::Assign(op, Box::new(left), Box::new(right)));
        }
        Ok(left)
    }

    fn parse_conditional(&mut self) -> Result<Expr, ParseError> {
        let test = self.parse_binary(0)?;
        if self.eat_punct("?") {
            let cons = self.parse_assign()?;
            self.expect_punct(":")?;
            let alt = self.parse_assign()?;
            return Ok(Expr::Conditional(Box::new(test), Box::new(cons), Box::new(alt)));
        }
        Ok(test)
    }

    fn parse_binary(&mut self, min_bp: u8) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let (op, bp, right_assoc, logical) = match self.peek() {
                Tok::Punct(p) => match p.as_str() {
                    "??" => (None, 1, false, Some(LogOp::Nullish)),
                    "||" => (None, 2, false, Some(LogOp::Or)),
                    "&&" => (None, 3, false, Some(LogOp::And)),
                    "|" => (Some(BinOp::BitOr), 4, false, None),
                    "^" => (Some(BinOp::BitXor), 5, false, None),
                    "&" => (Some(BinOp::BitAnd), 6, false, None),
                    "==" => (Some(BinOp::Eq), 7, false, None),
                    "!=" => (Some(BinOp::Ne), 7, false, None),
                    "===" => (Some(BinOp::StrictEq), 7, false, None),
                    "!==" => (Some(BinOp::StrictNe), 7, false, None),
                    "<" => (Some(BinOp::Lt), 8, false, None),
                    ">" => (Some(BinOp::Gt), 8, false, None),
                    "<=" => (Some(BinOp::Le), 8, false, None),
                    ">=" => (Some(BinOp::Ge), 8, false, None),
                    "<<" => (Some(BinOp::Shl), 9, false, None),
                    ">>" => (Some(BinOp::Shr), 9, false, None),
                    ">>>" => (Some(BinOp::UShr), 9, false, None),
                    "+" => (Some(BinOp::Add), 10, false, None),
                    "-" => (Some(BinOp::Sub), 10, false, None),
                    "*" => (Some(BinOp::Mul), 11, false, None),
                    "/" => (Some(BinOp::Div), 11, false, None),
                    "%" => (Some(BinOp::Mod), 11, false, None),
                    "**" => (Some(BinOp::Exp), 12, true, None),
                    _ => break,
                },
                Tok::Keyword(k) if k == "in" => {
                    if self.no_in {
                        break;
                    }
                    (Some(BinOp::In), 8, false, None)
                }
                Tok::Keyword(k) if k == "instanceof" => (Some(BinOp::Instanceof), 8, false, None),
                _ => break,
            };
            if bp < min_bp {
                break;
            }
            self.bump();
            let rhs = self.parse_binary(if right_assoc { bp } else { bp + 1 })?;
            lhs = if let Some(l) = logical {
                Expr::Logical(l, Box::new(lhs), Box::new(rhs))
            } else {
                Expr::Binary(op.unwrap(), Box::new(lhs), Box::new(rhs))
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        match self.peek().clone() {
            Tok::Punct(p) => match p.as_str() {
                "!" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Not, Box::new(self.parse_unary()?)))
                }
                "-" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Neg, Box::new(self.parse_unary()?)))
                }
                "+" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Pos, Box::new(self.parse_unary()?)))
                }
                "~" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::BitNot, Box::new(self.parse_unary()?)))
                }
                "++" | "--" => {
                    let inc = p == "++";
                    self.bump();
                    let target = self.parse_unary()?;
                    Ok(Expr::Update { inc, prefix: true, target: Box::new(target) })
                }
                _ => self.parse_postfix(),
            },
            Tok::Keyword(k) => match k.as_str() {
                "typeof" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Typeof, Box::new(self.parse_unary()?)))
                }
                "void" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Void, Box::new(self.parse_unary()?)))
                }
                "delete" => {
                    self.bump();
                    Ok(Expr::Unary(UnOp::Delete, Box::new(self.parse_unary()?)))
                }
                "await" => {
                    self.bump();
                    Ok(Expr::Await(Box::new(self.parse_unary()?)))
                }
                _ => self.parse_postfix(),
            },
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr, ParseError> {
        let mut expr = self.parse_new_expr()?;
        let mut ops: Vec<ChainOp> = Vec::new();
        let mut optional = false;
        loop {
            if self.eat_punct(".") {
                let name = self.parse_member_name()?;
                if optional {
                    ops.push(ChainOp::Member {
                        property: Box::new(Expr::String(name)),
                        computed: false,
                        optional: false,
                    });
                } else {
                    expr = Expr::Member {
                        object: Box::new(expr),
                        property: Box::new(Expr::String(name)),
                        computed: false,
                        optional: false,
                    };
                }
            } else if self.eat_punct("?.") {
                optional = true;
                if self.peek().is_punct("(") {
                    let args = self.parse_args()?;
                    ops.push(ChainOp::Call { args, optional: true });
                } else if self.eat_punct("[") {
                    let prop = self.parse_expr()?;
                    self.expect_punct("]")?;
                    ops.push(ChainOp::Member {
                        property: Box::new(prop),
                        computed: true,
                        optional: true,
                    });
                } else {
                    let name = self.parse_member_name()?;
                    ops.push(ChainOp::Member {
                        property: Box::new(Expr::String(name)),
                        computed: false,
                        optional: true,
                    });
                }
            } else if self.eat_punct("[") {
                let prop = self.parse_expr()?;
                self.expect_punct("]")?;
                if optional {
                    ops.push(ChainOp::Member { property: Box::new(prop), computed: true, optional: false });
                } else {
                    expr = Expr::Member { object: Box::new(expr), property: Box::new(prop), computed: true, optional: false };
                }
            } else if let Tok::Template { raw, exprs } = self.peek().clone() {
                if optional {
                    break;
                }
                self.bump();
                let exprs = self.parse_template_exprs(exprs)?;
                expr = Expr::TaggedTemplate { tag: Box::new(expr), raw, exprs };
            } else if self.peek().is_punct("(") {
                let args = self.parse_args()?;
                if optional {
                    ops.push(ChainOp::Call { args, optional: false });
                } else {
                    expr = Expr::Call { callee: Box::new(expr), args, optional: false };
                }
            } else {
                break;
            }
        }
        if optional {
            expr = Expr::Chain { base: Box::new(expr), ops };
        }
        if matches!(self.peek(), Tok::Punct(p) if p == "++" || p == "--") && !self.newline_before() {
            let inc = self.peek().is_punct("++");
            self.bump();
            return Ok(Expr::Update { inc, prefix: false, target: Box::new(expr) });
        }
        Ok(expr)
    }

    fn parse_member_name(&mut self) -> Result<String, ParseError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            Tok::Keyword(k) => {
                self.bump();
                Ok(k)
            }
            other => self.err(format!("expected a property name, found {}", other.describe())),
        }
    }

    fn parse_new_expr(&mut self) -> Result<Expr, ParseError> {
        if self.eat_kw("new") {
            if self.eat_punct(".") {
                let name = self.parse_member_name()?;
                if name == "target" {
                    return Ok(Expr::NewTarget);
                }
                return self.err(format!("expected 'target' after 'new.', found '{name}'"));
            }
            let mut callee = if self.peek().is_keyword("new") {
                self.parse_new_expr()?
            } else {
                self.parse_primary()?
            };
            loop {
                if self.eat_punct(".") {
                    let name = self.parse_member_name()?;
                    callee = Expr::Member {
                        object: Box::new(callee),
                        property: Box::new(Expr::String(name)),
                        computed: false,
                        optional: false,
                    };
                } else if self.eat_punct("[") {
                    let prop = self.parse_expr()?;
                    self.expect_punct("]")?;
                    callee = Expr::Member {
                        object: Box::new(callee),
                        property: Box::new(prop),
                        computed: true,
                        optional: false,
                    };
                } else {
                    break;
                }
            }
            let args = if self.peek().is_punct("(") { self.parse_args()? } else { Vec::new() };
            return Ok(Expr::New { callee: Box::new(callee), args });
        }
        self.parse_primary()
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, ParseError> {
        self.expect_punct("(")?;
        let mut args = Vec::new();
        while !self.peek().is_punct(")") {
            if self.eat_punct("...") {
                args.push(Expr::Spread(Box::new(self.parse_assign()?)));
            } else {
                args.push(self.parse_assign()?);
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(args)
    }

    fn parse_template_exprs(&mut self, exprs: Vec<String>) -> Result<Vec<Expr>, ParseError> {
        let mut parsed = Vec::new();
        for e in exprs {
            let mut sub = Parser {
                toks: tokenize(&e)
                    .map_err(|err| ParseError { message: err.message, line: err.line, offset: 0 })?,
                pos: 0,
                no_in: false,
            };
            let expr = sub.parse_expr()?;
            if !sub.at_eof() {
                return sub.err("unexpected token in template expression");
            }
            parsed.push(expr);
        }
        Ok(parsed)
    }

    fn parse_primary(&mut self) -> Result<Expr, ParseError> {
        let line = self.line();
        match self.peek().clone() {
            Tok::Num(n) => {
                self.bump();
                Ok(Expr::Number(n))
            }
            Tok::BigInt(n) => {
                self.bump();
                Ok(Expr::BigInt(n))
            }
            Tok::Str(s) => {
                self.bump();
                Ok(Expr::String(s))
            }
            Tok::Regex { source, flags } => {
                self.bump();
                Ok(Expr::Regex { source, flags })
            }
            Tok::Template { raw, exprs } => {
                self.bump();
                let parsed = self.parse_template_exprs(exprs)?;
                Ok(Expr::Template { raw, exprs: parsed })
            }
            Tok::Ident(name) => {
                if self.peek_at(1).is_keyword("function") && name == "async" {
                    self.bump();
                    return self.parse_function_expr(true);
                }
                self.bump();
                Ok(match name.as_str() {
                    "true" => Expr::Bool(true),
                    "false" => Expr::Bool(false),
                    "undefined" => Expr::Undefined,
                    _ => Expr::Ident(name),
                })
            }
            Tok::Keyword(k) => match k.as_str() {
                "true" => {
                    self.bump();
                    Ok(Expr::Bool(true))
                }
                "false" => {
                    self.bump();
                    Ok(Expr::Bool(false))
                }
                "null" => {
                    self.bump();
                    Ok(Expr::Null)
                }
                "this" => {
                    self.bump();
                    Ok(Expr::This)
                }
                "super" => {
                    self.bump();
                    if self.peek().is_punct("(") {
                        let args = self.parse_args()?;
                        Ok(Expr::SuperCall(args))
                    } else {
                        Ok(Expr::Super)
                    }
                }
                "function" => self.parse_function_expr(false),
                "async" if self.peek_at(1).is_keyword("function") => {
                    self.bump();
                    self.parse_function_expr(true)
                }
                "class" => self.parse_class_expr(),
                "import" => {
                    self.bump();
                    if self.eat_punct(".") {
                        if self.eat_kw("meta") || self.eat_ident("meta") {
                            return Ok(Expr::ImportMeta);
                        }
                        return self.err("expected `meta` after `import.`");
                    }
                    self.expect_punct("(")?;
                    let spec = self.parse_assign()?;
                    self.expect_punct(")")?;
                    Ok(Expr::DynamicImport(Box::new(spec)))
                }
                "new" => self.parse_new_expr(),
                "await" => {
                    self.bump();
                    Ok(Expr::Await(Box::new(self.parse_unary()?)))
                }
                _ => self.err(format!("unexpected keyword `{k}`")),
            },
            Tok::Punct(p) if p == "(" => {
                self.bump();
                let e = self.parse_expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Punct(p) if p == "[" => {
                self.bump();
                let mut items = Vec::new();
                while !self.peek().is_punct("]") {
                    if self.eat_punct(",") {
                        items.push(None);
                        continue;
                    }
                    if self.eat_punct("...") {
                        let e = self.parse_assign()?;
                        items.push(Some(Expr::Spread(Box::new(e))));
                    } else {
                        items.push(Some(self.parse_assign()?));
                    }
                    if !self.eat_punct(",") {
                        break;
                    }
                }
                self.expect_punct("]")?;
                Ok(Expr::Array(items))
            }
            Tok::Punct(p) if p == "{" => self.parse_object_literal(),
            other => self.err(format!("unexpected {}", other.describe())),
        }
        .map(|e| {
            let _ = line;
            e
        })
    }

    fn parse_object_literal(&mut self) -> Result<Expr, ParseError> {
        self.expect_punct("{")?;
        let mut props = Vec::new();
        while !self.peek().is_punct("}") {
            if self.eat_punct("...") {
                let value = self.parse_assign()?;
                props.push(ObjProp { key: PropKey::Static(String::new()), value: Some(value), kind: PropKind::Spread });
                if !self.eat_punct(",") {
                    break;
                }
                continue;
            }
            let mut kind = PropKind::Init;
            if matches!(self.peek(), Tok::Ident(s) if s == "get" || s == "set")
                && !self.peek_at(1).is_punct(":")
                && !self.peek_at(1).is_punct(",")
                && !self.peek_at(1).is_punct("}")
                && !self.peek_at(1).is_punct("(")
            {
                let is_get = matches!(self.peek(), Tok::Ident(s) if s == "get");
                self.bump();
                kind = if is_get { PropKind::Get } else { PropKind::Set };
            }
            let mut is_async = false;
            if matches!(self.peek(), Tok::Ident(s) if s == "async")
                && !self.peek_at(1).is_punct(":")
                && !self.peek_at(1).is_punct(",")
                && !self.peek_at(1).is_punct("}")
                && !self.peek_at(1).is_punct("(")
            {
                self.bump();
                is_async = true;
            }
            let is_generator = self.eat_punct("*");
            let key = self.parse_prop_key()?;
            if self.peek().is_punct("(") {
                let (params, body) = self.parse_function_rest()?;
                let name = match &key {
                    PropKey::Static(s) => Some(s.clone()),
                    _ => None,
                };
                let value = Expr::Function(Rc::new(FnNode {
                    name,
                    params,
                    body,
                    is_arrow: false,
                    is_async,
                    is_generator,
                    self_bind: false,
                    line: self.line()
                }));
                props.push(ObjProp { key, value: Some(value), kind });
            } else if self.eat_punct(":") {
                let value = self.parse_assign()?;
                props.push(ObjProp { key, value: Some(value), kind });
            } else if matches!(kind, PropKind::Get | PropKind::Set) {
                return self.err("accessors require a body");
            } else {
                let name = match &key {
                    PropKey::Static(s) => s.clone(),
                    _ => return self.err("computed shorthand properties need a value"),
                };
                props.push(ObjProp {
                    key,
                    value: Some(Expr::Ident(name)),
                    kind: PropKind::Init,
                });
            }
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct("}")?;
        Ok(Expr::Object(props))
    }

    fn parse_function_expr(&mut self, is_async: bool) -> Result<Expr, ParseError> {
        let line = self.line();
        self.expect_kw("function")?;
        let is_generator = self.eat_punct("*");
        let name = if matches!(self.peek(), Tok::Ident(_)) {
            Some(self.ident_name()?)
        } else {
            None
        };
        let (params, body) = self.parse_function_rest()?;
        Ok(Expr::Function(Rc::new(FnNode {
            name,
            params,
            body,
            is_arrow: false,
            is_async,
            is_generator,
            self_bind: true,
            line,
        })))
    }

    fn parse_class_expr(&mut self) -> Result<Expr, ParseError> {
        self.expect_kw("class")?;
        let name = if matches!(self.peek(), Tok::Ident(_)) {
            Some(self.ident_name()?)
        } else {
            None
        };
        Ok(Expr::Class(Rc::new(self.parse_class_tail(name)?)))
    }

    fn arrow_ahead(&self) -> bool {
        if let Tok::Ident(name) = self.peek() {
            if self.peek_at(1).is_punct("=>") {
                return true;
            }
            if name == "async" {
                if matches!(self.peek_at(1), Tok::Ident(_)) && self.peek_at(2).is_punct("=>") {
                    return true;
                }
                if self.peek_at(1).is_punct("(") {
                    return self.paren_arrow(1);
                }
            }
            return false;
        }
        if self.peek().is_punct("(") {
            return self.paren_arrow(0);
        }
        false
    }

    fn paren_arrow(&self, start: usize) -> bool {
        let mut depth = 0i32;
        let mut i = self.pos + start;
        while i < self.toks.len() {
            match &self.toks[i].kind {
                Tok::Punct(p) if p == "(" => depth += 1,
                Tok::Punct(p) if p == ")" => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(&self.toks.get(i + 1).map(|t| &t.kind), Some(Tok::Punct(p)) if p == "=>");
                    }
                }
                Tok::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn parse_arrow(&mut self) -> Result<Expr, ParseError> {
        let line = self.line();
        let mut is_async = false;
        if matches!(self.peek(), Tok::Ident(s) if s == "async")
            && (self.peek_at(1).is_punct("(") || matches!(self.peek_at(1), Tok::Ident(_)))
        {
            is_async = true;
            self.bump();
        }
        let mut params = Vec::new();
        if self.eat_punct("(") {
            while !self.peek().is_punct(")") {
                if self.eat_punct("...") {
                    params.push(Pattern::Rest(Box::new(self.parse_pattern()?)));
                    break;
                }
                params.push(self.parse_pattern()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(")")?;
        } else {
            params.push(Pattern::Ident(self.ident_name()?));
        }
        self.expect_punct("=>")?;
        let body = if self.peek().is_punct("{") {
            self.bump();
            FnBody::Block(self.parse_block_body()?)
        } else {
            FnBody::Expr(self.parse_assign()?)
        };
        Ok(Expr::Function(Rc::new(FnNode {
            name: None,
            params,
            body,
            is_arrow: true,
            is_async,
            is_generator: false,
            self_bind: false,
            line,
        })))
    }
}

fn expr_to_pattern(e: Expr) -> Pattern {
    match e {
        Expr::Ident(name) => Pattern::Ident(name),
        Expr::Array(items) => Pattern::Array {
            items: items.into_iter().map(|i| i.map(expr_to_pattern)).collect(),
            rest: None,
        },
        Expr::Object(props) => Pattern::Object {
            props: props
                .into_iter()
                .filter_map(|p| match p.key {
                    PropKey::Static(name) => Some((
                        PropKey::Static(name.clone()),
                        p.value.map(expr_to_pattern).or(Some(Pattern::Ident(name))),
                        None,
                    )),
                    PropKey::Computed(e) => Some((PropKey::Computed(e), p.value.map(expr_to_pattern), None)),
                })
                .collect(),
            rest: None,
        },
        other => Pattern::Ident(format!("{:?}", other)),
    }
}

fn number_key(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variables_and_functions() {
        let stmts = parse_program("let x = 1; const f = (a, b = 2) => a + b; function g() { return f(x); }").unwrap();
        assert_eq!(stmts.len(), 3);
        assert!(matches!(stmts[0], Stmt::Var { kind: VarKind::Let, .. }));
    }

    #[test]
    fn parses_objects_classes_and_templates() {
        let src = r#"
            class A extends B {
                constructor(x) { super(x); this.x = x; }
                get value() { return this.x; }
                static make() { return new A(1); }
            }
            const o = { a: 1, get b() { return 2 }, m() {}, ...rest };
            const s = `hi ${o.a + 1}`;
            for (const k of [1,2,3]) { console.log(k); }
        "#;
        let stmts = parse_program(src).unwrap();
        assert!(stmts.iter().any(|s| matches!(s, Stmt::ClassDecl(_))));
    }

    #[test]
    fn parses_control_flow_and_try() {
        let src = "for (let i=0;i<10;i++) { if (i%2) continue; else break; } try { throw new Error('x') } catch (e) { } finally { } switch (x) { case 1: break; default: }";
        parse_program(src).unwrap();
    }

    #[test]
    fn parses_nested_arrow_arguments() {
        for src in [
            "f(() => {}, 0);",
            "f(() => { out = 't'; }, 0);",
            "f(() => { out += 't'; }, 0);",
            "f(() => { out; }, 0);",
            "f(() => { out += 1; }, 0);",
            "setTimeout(() => { out += 't'; }, 0);",
            "fetch('/a').then(r => r.json());",
            "fetch('/a').then(r => r.json()).then(j => { x = j.n; });",
        ] {
            if let Err(e) = parse_program(src) {
                panic!("failed on {src}: {e:?}");
            }
        }
    }

    #[test]
    fn parses_module_import_export_and_dynamic_import() {
        let src = r#"
            import def, { a as b, c } from './lib.js';
            import * as ns from './ns.js';
            import './side.js';
            export const x = 1;
            export function f() { return import('./lazy.js'); }
            export { b as bee };
            export * from './all.js';
            export default 42;
            const m = import.meta.url;
        "#;
        let module = parse_module(src).unwrap();
        assert_eq!(module.imports.len(), 3);
        assert_eq!(module.imports[0].default.as_deref(), Some("def"));
        assert_eq!(module.imports[0].named, vec![("a".to_string(), "b".to_string()), ("c".to_string(), "c".to_string())]);
        assert_eq!(module.imports[1].namespace.as_deref(), Some("ns"));
        assert!(module.exports.iter().any(|(e, l)| e == "x" && l == "x"));
        assert!(module.exports.iter().any(|(e, l)| e == "bee" && l == "b"));
        assert!(module.exports.iter().any(|(e, _)| e == "default"));
        assert_eq!(module.export_all, vec!["./all.js".to_string()]);
        assert!(matches!(
            module.body.iter().find(|s| matches!(s, Stmt::FuncDecl(_))),
            Some(Stmt::FuncDecl(_))
        ));
    }

    #[test]
    fn parses_chains_and_arrows() {
        let e = parse_expression("a?.b?.[c]?.(1) ?? d.e").unwrap();
        assert!(matches!(e, Expr::Logical(LogOp::Nullish, _, _)));
        let e = parse_expression("async (x) => await x").unwrap();
        assert!(matches!(e, Expr::Function(_)));
    }
}
