//! Abstract Syntax Tree for SQL queries
/*
An Abstract Syntax Tree (AST) for an SQL database is a structured, tree-shaped representation 
of an SQL query after it has been parsed, but before it is executed.
*/

use crate::types::{ColumnDef, Value};

/// SQL statement types
#[derive(Debug, Clone)]
pub enum Statement {
    CreateTable(CreateTableStmt),
    Insert(InsertStmt),
    Select(SelectStmt),
    Delete(DeleteStmt),
    BeginTransaction,
    Commit,
    Rollback,
}

/// CREATE TABLE statement
#[derive(Debug, Clone)]
pub struct CreateTableStmt {
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
    pub if_not_exists: bool,
}

/// INSERT statement
#[derive(Debug, Clone)]
pub struct InsertStmt {
    pub table_name: String,
    pub columns: Option<Vec<String>>,
    pub values: Vec<Vec<Expr>>,
}

/// SELECT statement
#[derive(Debug, Clone)]
pub struct SelectStmt {
    pub distinct: bool,
    pub columns: Vec<SelectItem>,
    pub from: TableRef,
    pub where_clause: Option<Expr>,
    pub order_by: Vec<OrderByExpr>,
    pub limit: Option<Expr>,
    pub offset: Option<Expr>,
}

/// DELETE statement
#[derive(Debug, Clone)]
pub struct DeleteStmt {
    pub table_name: String,
    pub where_clause: Option<Expr>,
}

/// Expression types
#[derive(Debug, Clone)]
pub enum Expr {
    Literal(Value),
    ColumnRef(String),
    BinaryExpr {
        left: Box<Expr>,
        op: BinaryOperator,
        right: Box<Expr>,
    },
    UnaryExpr {
        op: UnaryOperator,
        expr: Box<Expr>,
    },
    FunctionCall {
        name: String,
        args: Vec<Expr>,
    },
    IsNull(Box<Expr>),
    IsNotNull(Box<Expr>),
    Between {
        expr: Box<Expr>,
        low: Box<Expr>,
        high: Box<Expr>,
    },
    InList {
        expr: Box<Expr>,
        list: Vec<Expr>,
    },
}

/// Binary operators
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinaryOperator {
    Eq,
    Neq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
    Add,
    Sub,
    Mul,
    Div,
    Like,
}

/// Unary operators
#[derive(Debug, Clone, Copy)]
pub enum UnaryOperator {
    Not,
    Neg,
}

/// SELECT items
#[derive(Debug, Clone)]
pub enum SelectItem {
    Wildcard,
    Expr { expr: Expr, alias: Option<String> },
}

/// Table references
#[derive(Debug, Clone)]
pub enum TableRef {
    Table {
        name: String,
        alias: Option<String>,
    },
    // TODO: Add joins, subqueries
}

/// ORDER BY expressions
#[derive(Debug, Clone)]
pub struct OrderByExpr {
    pub expr: Expr,
    pub asc: bool,
}