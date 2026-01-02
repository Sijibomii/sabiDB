pub mod ast;

use sqlparser::{dialect::GenericDialect, parser::Parser};
use crate::types::{ColumnDef, DataType, Value};
pub use ast::*;
use crate::error::SqlError;

/// SQL Parser - converts SQL strings to AST
pub struct QueryParser {
    dialect: GenericDialect,
}

impl Default for QueryParser {
    fn default() -> Self {
        Self::new()
    }
}

impl QueryParser {
    pub fn new() -> Self {
        Self {
            dialect: GenericDialect::default(),
        }
    }
    
    /// Parse a SQL string into AST statements
    pub fn parse(&self, sql: &str) -> Result<Vec<Statement>, SqlError> {
        // use the sql parser library to parse SQL and generate statements
        let statements = Parser::parse_sql(&self.dialect, sql)?;
        
        let mut ast_statements = Vec::new();
        for stmt in statements {
            // convert the sqlparser AST to our internal AST
            ast_statements.push(self.convert_statement(stmt)?);
        }
        
        Ok(ast_statements)
    }
    
    /// Convert sqlparser AST to our internal AST
    fn convert_statement(&self, stmt: sqlparser::ast::Statement) -> Result<Statement, SqlError> {
    match stmt {
        sqlparser::ast::Statement::CreateTable(create_table) => {
            Ok(Statement::CreateTable(self.parse_create_table(
                create_table.name,
                create_table.columns,
            )?))
        }
        
        sqlparser::ast::Statement::Insert(insert_stmt) =>  {
            // Extract table name from TableObject enum
            let table_name = match insert_stmt.table {
                sqlparser::ast::TableObject::TableName(name) => name,
                sqlparser::ast::TableObject::TableFunction(_) => {
                    return Err(SqlError::ParseError(
                        "INSERT INTO table functions not supported".into()
                    ));
                }
            };
            
            Ok(Statement::Insert(self.parse_insert(
                table_name,
                insert_stmt.columns, 
                *insert_stmt.source.unwrap()
            )?))
        }
        
        sqlparser::ast::Statement::Query(query) => {
            Ok(Statement::Select(self.parse_select(*query)?))
        }
        
        sqlparser::ast::Statement::Delete(delete_stmt) => {
            // Delete can have multiple tables, handle them
            // For simplicity, we'll use the first table

            // Get the first table (main table to delete from)
            let table = delete_stmt.tables.first()
                .ok_or_else(|| SqlError::ParseError("No table specified in DELETE statement".into()))?;
            
            Ok(Statement::Delete(self.parse_delete(table.clone(), delete_stmt.selection)?))
        }
        
        sqlparser::ast::Statement::StartTransaction { .. } => {
            Ok(Statement::BeginTransaction)
        }
        
        sqlparser::ast::Statement::Commit { .. } => {
            Ok(Statement::Commit)
        }
        
        sqlparser::ast::Statement::Rollback { .. } => {
            Ok(Statement::Rollback)
        }
        
        _ => Err(SqlError::ParseError(format!("Unsupported statement: {:?}", stmt))),
    }
}
    
    fn parse_create_table(
        &self,
        name: sqlparser::ast::ObjectName,
        columns: Vec<sqlparser::ast::ColumnDef>,
    ) -> Result<CreateTableStmt, SqlError> {
        let table_name = self.object_name_to_string(name)?;
        
        let mut column_defs = Vec::new();
        for col in columns {
            let data_type = self.convert_data_type(&col.data_type)?;
            let nullable = !col.options.iter().any(|opt| {
                matches!(opt.option, sqlparser::ast::ColumnOption::NotNull)
            });
            
            let primary_key = col.options.iter().any(|opt| {
                matches!(opt.option, sqlparser::ast::ColumnOption::PrimaryKey(_))
            });
            
            column_defs.push(crate::types::ColumnDef {
                name: col.name.value,
                data_type,
                nullable,
                primary_key,
            });
        }
        
        Ok(CreateTableStmt {
            table_name,
            columns: column_defs,
            if_not_exists: false, // TODO: Parse this from statement
        })
    }
    
    fn parse_insert(
        &self,
        table_name: sqlparser::ast::ObjectName,
        columns: Vec<sqlparser::ast::Ident>,
        source: sqlparser::ast::Query,
    ) -> Result<InsertStmt, SqlError> {
        let table_name = self.object_name_to_string(table_name)?;
        
        let column_names = if columns.is_empty() {
            None
        } else {
            Some(columns.into_iter().map(|c| c.value).collect())
        };
        
        // Parse values from query
        let values = match *source.body {
            sqlparser::ast::SetExpr::Values(sqlparser::ast::Values { rows, .. }) => {
                let mut value_rows = Vec::new();
                for row in rows {
                    let mut exprs = Vec::new();
                    for expr in row {
                        exprs.push(self.convert_expr(expr)?);
                    }
                    value_rows.push(exprs);
                }
                value_rows
            }
            _ => return Err(SqlError::ParseError("INSERT must have VALUES clause".into())),
        };
        
        Ok(InsertStmt {
            table_name,
            columns: column_names,
            values,
        })
    }
    

    fn parse_select(&self, query: sqlparser::ast::Query) -> Result<SelectStmt, SqlError> {
        let body = *query.body;
        let select = match body {
            sqlparser::ast::SetExpr::Select(select) => select,
            _ => return Err(SqlError::ParseError("Only SELECT queries supported".into())),
        };
        
        // Parse SELECT items
        let mut columns = Vec::new();
        for item in select.projection {
            match item {
                sqlparser::ast::SelectItem::Wildcard(_) => {
                    columns.push(SelectItem::Wildcard);
                }
                sqlparser::ast::SelectItem::UnnamedExpr(expr) => {
                    columns.push(SelectItem::Expr {
                        expr: self.convert_expr(expr)?,
                        alias: None,
                    });
                }
                sqlparser::ast::SelectItem::ExprWithAlias { expr, alias } => {
                    columns.push(SelectItem::Expr {
                        expr: self.convert_expr(expr)?,
                        alias: Some(alias.value),
                    });
                }
                _ => return Err(SqlError::ParseError("Unsupported SELECT item".into())),
            }
        }
        
        // Parse FROM clause
        let from = if let Some(table) = select.from.first() {
            self.parse_table_ref(&table.relation)?
        } else {
            return Err(SqlError::ParseError("SELECT must have FROM clause".into()));
        };
        
        // Parse WHERE clause
        let where_clause = select.selection.map(|expr| self.convert_expr(expr)).transpose()?;
        
        // Parse ORDER BY
        let mut order_by = Vec::new();
        
        for order in query.order_by {
            match order.kind {
                sqlparser::ast::OrderByKind::Expressions(exprs) => {
                    for expr in exprs {
                        order_by.push(OrderByExpr {
                            expr: self.convert_expr(expr.expr)?,
                            asc: expr.options.asc.unwrap_or(true), // true = ASC, false = DESC
                        });
                    }
                }
                _ => return Err(SqlError::ParseError("Unsupported ORDER BY kind".into())),
            }
        }
        
        // Parse LIMIT/OFFSET
        let (limit, offset) = match query.limit_clause {
            Some(limit_clause) => match limit_clause {
                sqlparser::ast::LimitClause::LimitOffset {
                    limit,
                    offset,
                    limit_by: _,
                } => {
                    let limit = limit
                        .map(|e| self.convert_expr(e))
                        .transpose()?
                        .ok_or_else(|| {
                            SqlError::ParseError("LIMIT must have a value".into())
                        })?;

                    let offset = offset
                        .map(|e| self.convert_expr(e.value))
                        .transpose()?;

                    (Some(limit), offset)
                }

                sqlparser::ast::LimitClause::OffsetCommaLimit { offset, limit } => {
                    let limit = self.convert_expr(limit)?;
                    let offset = self.convert_expr(offset)?;
                    (Some(limit), Some(offset))
                }
            },
            None => (None, None)
        };


        Ok(SelectStmt {
            distinct: select.distinct.is_some(),
            columns,
            from,
            where_clause,
            order_by,
            limit,
            offset,
        })
    }
        
    fn parse_delete(
        &self,
        table_name: sqlparser::ast::ObjectName,
        selection: Option<sqlparser::ast::Expr>,
    ) -> Result<DeleteStmt, SqlError> {
        let table_name = self.object_name_to_string(table_name)?;
        let where_clause = selection.map(|expr| self.convert_expr(expr)).transpose()?;
        
        Ok(DeleteStmt {
            table_name,
            where_clause,
        })
    }
    
    fn parse_table_ref(&self, relation: &sqlparser::ast::TableFactor) -> Result<TableRef, SqlError> {
        match relation {
            sqlparser::ast::TableFactor::Table { name, alias, .. } => {
                let table_name = self.object_name_to_string(name.clone())?;
                let alias = alias.as_ref().map(|a| a.name.value.clone());
                Ok(TableRef::Table { name: table_name, alias })
            }
            _ => Err(SqlError::ParseError("Unsupported table reference".into())),
        }
    }
    
    fn convert_expr(&self, expr: sqlparser::ast::Expr) -> Result<Expr, SqlError> {
        match expr {
            sqlparser::ast::Expr::Value(value_with_span) => {
                let val = match value_with_span.value {
                    sqlparser::ast::Value::Number(s, _) => {
                        Value::Integer(s.parse().map_err(|e| {
                            SqlError::ParseError(format!("Invalid number: {}", e))
                        })?)
                    }
                    sqlparser::ast::Value::SingleQuotedString(s) => Value::Text(s),
                    sqlparser::ast::Value::Boolean(b) => Value::Boolean(b),
                    sqlparser::ast::Value::Null => Value::Null,
                    _ => return Err(SqlError::ParseError("Unsupported value type".into())),
                };
                Ok(Expr::Literal(val))
            }
            
            sqlparser::ast::Expr::Identifier(ident) => {
                Ok(Expr::ColumnRef(ident.value))
            }
            
            sqlparser::ast::Expr::BinaryOp { left, op, right } => {
                let op = match op {
                    sqlparser::ast::BinaryOperator::Eq => BinaryOperator::Eq,
                    sqlparser::ast::BinaryOperator::NotEq => BinaryOperator::Neq,
                    sqlparser::ast::BinaryOperator::Lt => BinaryOperator::Lt,
                    sqlparser::ast::BinaryOperator::LtEq => BinaryOperator::LtEq,
                    sqlparser::ast::BinaryOperator::Gt => BinaryOperator::Gt,
                    sqlparser::ast::BinaryOperator::GtEq => BinaryOperator::GtEq,
                    sqlparser::ast::BinaryOperator::And => BinaryOperator::And,
                    sqlparser::ast::BinaryOperator::Or => BinaryOperator::Or,
                    sqlparser::ast::BinaryOperator::Plus => BinaryOperator::Add,
                    sqlparser::ast::BinaryOperator::Minus => BinaryOperator::Sub,
                    sqlparser::ast::BinaryOperator::Multiply => BinaryOperator::Mul,
                    sqlparser::ast::BinaryOperator::Divide => BinaryOperator::Div,
                    // sqlparser::ast::BinaryOperator::Like => BinaryOperator::Like,
                    _ => return Err(SqlError::ParseError("Unsupported binary operator".into())),
                };
                
                Ok(Expr::BinaryExpr {
                    left: Box::new(self.convert_expr(*left)?),
                    op,
                    right: Box::new(self.convert_expr(*right)?),
                })
            }
            
            sqlparser::ast::Expr::UnaryOp { op, expr } => {
                let op = match op {
                    sqlparser::ast::UnaryOperator::Not => UnaryOperator::Not,
                    sqlparser::ast::UnaryOperator::Minus => UnaryOperator::Neg,
                    _ => return Err(SqlError::ParseError("Unsupported unary operator".into())),
                };
                
                Ok(Expr::UnaryExpr {
                    op,
                    expr: Box::new(self.convert_expr(*expr)?),
                })
            }
            
            sqlparser::ast::Expr::IsNull(expr) => {
                Ok(Expr::IsNull(Box::new(self.convert_expr(*expr)?)))
            }
            
            sqlparser::ast::Expr::IsNotNull(expr) => {
                Ok(Expr::IsNotNull(Box::new(self.convert_expr(*expr)?)))
            }
            
            sqlparser::ast::Expr::Between {
                expr,
                negated,
                low,
                high,
            } => {
                if negated {
                    // Convert NOT BETWEEN to NOT (expr BETWEEN low AND high)
                    let between = Expr::Between {
                        expr: Box::new(self.convert_expr(*expr)?),
                        low: Box::new(self.convert_expr(*low)?),
                        high: Box::new(self.convert_expr(*high)?),
                    };
                    Ok(Expr::UnaryExpr {
                        op: UnaryOperator::Not,
                        expr: Box::new(between),
                    })
                } else {
                    Ok(Expr::Between {
                        expr: Box::new(self.convert_expr(*expr)?),
                        low: Box::new(self.convert_expr(*low)?),
                        high: Box::new(self.convert_expr(*high)?),
                    })
                }
            }
            
            _ => Err(SqlError::ParseError(format!("Unsupported expression: {:?}", expr))),
        }
    }
    
    fn convert_data_type(&self, data_type: &sqlparser::ast::DataType) -> Result<DataType, SqlError> {
        match data_type {
            sqlparser::ast::DataType::Int(_) | sqlparser::ast::DataType::Integer(_) => {
                Ok(DataType::Integer)
            }
            sqlparser::ast::DataType::Text | sqlparser::ast::DataType::Varchar(_) => {
                Ok(DataType::Text)
            }
            sqlparser::ast::DataType::Boolean => Ok(DataType::Boolean),
            _ => Err(SqlError::ParseError(format!("Unsupported data type: {:?}", data_type))),
        }
    }
    
    fn object_name_to_string(
        &self,
        name: sqlparser::ast::ObjectName,
    ) -> Result<String, SqlError> {
        let parts: Vec<String> = name
            .0
            .into_iter()
            .map(|part| match part {
                sqlparser::ast::ObjectNamePart::Identifier(ident) => Ok(ident.value),
                _ => {
                    return Err(SqlError::ParseError(
                        "Unsupported object name part".into(),
                    ))
                }
            })
            .collect::<Result<_, _>>()?;

        if parts.len() != 1 {
            return Err(SqlError::ParseError(
                "Only simple table names supported".into(),
            ));
        }

        Ok(parts[0].clone())
    }

}