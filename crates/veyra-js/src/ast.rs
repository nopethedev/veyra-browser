//! JavaScript abstract syntax tree.

use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Not,
    Neg,
    Pos,
    BitNot,
    Typeof,
    Void,
    Delete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Gt,
    Le,
    Ge,
    In,
    Instanceof,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogOp {
    And,
    Or,
    Nullish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssignOp {
    Assign,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
    And,
    Or,
    Nullish,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VarKind {
    Var,
    Let,
    Const,
}

#[derive(Clone, Debug)]
pub enum PropKey {
    Static(String),
    Computed(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropKind {
    Init,
    Method,
    Get,
    Set,
    Spread,
}

#[derive(Clone, Debug)]
pub struct ObjProp {
    pub key: PropKey,
    pub value: Option<Expr>,
    pub kind: PropKind,
}

#[derive(Clone, Debug)]
pub struct FnNode {
    pub name: Option<String>,
    pub params: Vec<Pattern>,
    pub body: FnBody,
    pub is_arrow: bool,
    pub is_async: bool,
    pub is_generator: bool,
    /// A named function expression binds its own name inside its body.
    pub self_bind: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
pub enum FnBody {
    Block(Vec<Stmt>),
    Expr(Expr),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberKind {
    Method,
    Get,
    Set,
    Field,
    StaticBlock,
}

#[derive(Clone, Debug)]
pub struct ClassMember {
    pub key: PropKey,
    pub value: Option<Expr>,
    pub kind: MemberKind,
    pub is_static: bool,
}

#[derive(Clone, Debug)]
pub struct ClassNode {
    pub name: Option<String>,
    pub super_class: Option<Expr>,
    pub members: Vec<ClassMember>,
}

#[derive(Clone, Debug)]
pub enum Pattern {
    Ident(String),
    Array {
        items: Vec<Option<Pattern>>,
        rest: Option<Box<Pattern>>,
    },
    Object {
        props: Vec<(PropKey, Option<Pattern>, Option<Expr>)>,
        rest: Option<Box<Pattern>>,
    },
    Default(Box<Pattern>, Box<Expr>),
    Rest(Box<Pattern>),
}

#[derive(Clone, Debug)]
pub enum ChainOp {
    Member { property: Box<Expr>, computed: bool, optional: bool },
    Call { args: Vec<Expr>, optional: bool },
}

#[derive(Clone, Debug)]
pub enum Expr {
    Number(f64),
    BigInt(i128),
    String(String),
    Bool(bool),
    Null,
    Undefined,
    Ident(String),
    This,
    NewTarget,
    Super,
    Array(Vec<Option<Expr>>),
    Object(Vec<ObjProp>),
    Function(Rc<FnNode>),
    Class(Rc<ClassNode>),
    Template { raw: Vec<String>, exprs: Vec<Expr> },
    TaggedTemplate { tag: Box<Expr>, raw: Vec<String>, exprs: Vec<Expr> },
    Unary(UnOp, Box<Expr>),
    Update { inc: bool, prefix: bool, target: Box<Expr> },
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Logical(LogOp, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Assign(AssignOp, Box<Expr>, Box<Expr>),
    Member { object: Box<Expr>, property: Box<Expr>, computed: bool, optional: bool },
    Chain { base: Box<Expr>, ops: Vec<ChainOp> },
    Call { callee: Box<Expr>, args: Vec<Expr>, optional: bool },
    New { callee: Box<Expr>, args: Vec<Expr> },
    Seq(Vec<Expr>),
    Spread(Box<Expr>),
    Regex { source: String, flags: String },
    Await(Box<Expr>),
    Yield { value: Option<Box<Expr>>, delegate: bool },
    DynamicImport(Box<Expr>),
    ImportMeta,
    SuperCall(Vec<Expr>),
}

#[derive(Clone, Debug)]
pub struct ImportBinding {
    pub source: String,
    pub default: Option<String>,
    pub namespace: Option<String>,
    pub named: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct ModuleAst {
    pub body: Vec<Stmt>,
    pub imports: Vec<ImportBinding>,
    /// Export name -> local binding name.
    pub exports: Vec<(String, String)>,
    /// (source, imported name, exported name).
    pub reexports: Vec<(String, String, String)>,
    /// `export * from "source"`.
    pub export_all: Vec<String>,
}

#[derive(Clone, Debug)]
pub enum ForLeft {
    Var(VarKind, Pattern),
    Pat(Pattern),
}

#[derive(Clone, Debug)]
pub enum Stmt {
    /// Source-located wrapper used for diagnostics. Execution and hoisting
    /// both delegate to the inner statement.
    Located { line: u32, stmt: Box<Stmt> },
    Expr(Expr),
    Block(Vec<Stmt>),
    Empty,
    Var { kind: VarKind, decls: Vec<(Pattern, Option<Expr>)> },
    FuncDecl(Rc<FnNode>),
    ClassDecl(Rc<ClassNode>),
    If { test: Expr, cons: Box<Stmt>, alt: Option<Box<Stmt>> },
    For { init: Option<Box<Stmt>>, test: Option<Expr>, update: Option<Expr>, body: Box<Stmt> },
    ForIn { left: ForLeft, right: Expr, body: Box<Stmt>, of: bool },
    While { test: Expr, body: Box<Stmt> },
    DoWhile { body: Box<Stmt>, test: Expr },
    Return(Option<Expr>),
    Throw(Expr),
    Try { body: Box<Stmt>, catch: Option<(Option<Pattern>, Box<Stmt>)>, finally: Option<Box<Stmt>> },
    Switch { disc: Expr, cases: Vec<(Option<Expr>, Vec<Stmt>)> },
    Break(Option<String>),
    Continue(Option<String>),
    Debugger,
    Labeled { label: String, body: Box<Stmt> },
}
