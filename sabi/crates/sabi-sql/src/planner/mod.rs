use crate::parser::ast::*;
use crate::types::{TableSchema, Value};
use crate::error::SqlError;

/// Logical query plan nodes
#[derive(Debug)]
pub enum LogicalPlan {
    /// Scan a table
    Scan {
        table_name: String,
        columns: Vec<String>,
        filter: Option<Expr>,
        schema: TableSchema,
    },
    
    /// Project specific columns
    Project {
        input: Box<LogicalPlan>,
        columns: Vec<String>,
        expressions: Vec<Expr>,
    },
    
    /// Filter rows
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    
    /// Limit results
    Limit {
        input: Box<LogicalPlan>,
        limit: usize,
        offset: usize,
    },
    
    /// Create table
    CreateTable {
        schema: TableSchema,
        if_not_exists: bool,
    },
    
    /// Insert rows
    Insert {
        table_name: String,
        columns: Option<Vec<String>>,
        values: Vec<Vec<Expr>>,
        schema: TableSchema,
    },
    
    /// Delete rows
    Delete {
        table_name: String,
        filter: Option<Expr>,
        schema: TableSchema,
    },
    
    /// Transaction operations
    BeginTransaction,
    Commit,
    Rollback,
}

/// Query planner - converts AST to logical plan
pub struct QueryPlanner {
    /// Catalog of tables and schemas
    pub catalog: Catalog,
}

/// Simple catalog for table metadata
pub struct Catalog {
    tables: std::collections::HashMap<String, TableSchema>,
}

impl Catalog {
    pub fn new() -> Self {
        Self {
            tables: std::collections::HashMap::new(),
        }
    }
    
    pub fn add_table(&mut self, schema: TableSchema) {
        self.tables.insert(schema.name.clone(), schema);
    }
    
    pub fn get_table(&self, name: &str) -> Option<&TableSchema> {
        self.tables.get(name)
    }
    
    pub fn table_exists(&self, name: &str) -> bool {
        self.tables.contains_key(name)
    }
}

impl QueryPlanner {
    pub fn new(catalog: Catalog) -> Self {
        Self { catalog }
    }
    
    /// Plan a single statement
    pub fn plan(&self, stmt: Statement) -> Result<LogicalPlan, SqlError> {
        match stmt {
            Statement::CreateTable(create) => self.plan_create_table(create),
            Statement::Insert(insert) => self.plan_insert(insert),
            Statement::Select(select) => self.plan_select(select),
            Statement::Delete(delete) => self.plan_delete(delete),
            Statement::BeginTransaction => Ok(LogicalPlan::BeginTransaction),
            Statement::Commit => Ok(LogicalPlan::Commit),
            Statement::Rollback => Ok(LogicalPlan::Rollback),
        }
    }
    
    fn plan_create_table(&self, stmt: CreateTableStmt) -> Result<LogicalPlan, SqlError> {
        // Check if table already exists
        if !stmt.if_not_exists && self.catalog.table_exists(&stmt.table_name) {
            return Err(SqlError::PlannerError(format!(
                "Table '{}' already exists", stmt.table_name
            )));
        }
        
        let schema = TableSchema {
            name: stmt.table_name.clone(),
            columns: stmt.columns.clone(),
            primary_key: stmt.columns.iter()
                .filter(|c| c.primary_key)
                .map(|c| c.name.clone())
                .collect::<Vec<_>>()
                .into(),
        };
        
        Ok(LogicalPlan::CreateTable {
            schema,
            if_not_exists: stmt.if_not_exists,
        })
    }
    
    fn plan_insert(&self, stmt: InsertStmt) -> Result<LogicalPlan, SqlError> {
        // Get table schema
        let schema = self.catalog.get_table(&stmt.table_name)
            .ok_or_else(|| SqlError::TableNotFound(stmt.table_name.clone()))?
            .clone();
        
        // Validate column count
        if let Some(cols) = &stmt.columns {
            if cols.len() != stmt.values[0].len() {
                return Err(SqlError::PlannerError(
                    "Column count doesn't match value count".into()
                ));
            }
            
            // Verify all columns exist
            for col in cols {
                if !schema.columns.iter().any(|c| &c.name == col) {
                    return Err(SqlError::ColumnNotFound(col.clone()));
                }
            }
        } else {
            // Inserting into all columns
            if stmt.values[0].len() != schema.columns.len() {
                return Err(SqlError::PlannerError(
                    "Value count doesn't match column count".into()
                ));
            }
        }
        
        Ok(LogicalPlan::Insert {
            table_name: stmt.table_name,
            columns: stmt.columns,
            values: stmt.values,
            schema,
        })
    }
    
    fn plan_select(&self, stmt: SelectStmt) -> Result<LogicalPlan, SqlError> {
        // Get table name and schema
        let (table_name, schema) = match stmt.from {
            TableRef::Table { name, .. } => {
                let schema = self.catalog.get_table(&name)
                    .ok_or_else(|| SqlError::TableNotFound(name.clone()))?
                    .clone();
                (name, schema)
            }
        };
        
        // Build column list
        let columns = self.resolve_select_items(&stmt.columns, &schema)?;
        
        // Start with scan
        let mut plan = LogicalPlan::Scan {
            table_name,
            columns: columns.clone(),
            filter: stmt.where_clause.clone(),
            schema: schema.clone(),
        };
        
        // Apply WHERE filter
        if let Some(predicate) = stmt.where_clause {
            plan = LogicalPlan::Filter {
                input: Box::new(plan),
                predicate,
            };
        }
        
        // Apply LIMIT and OFFSET
        if stmt.limit.is_some() || stmt.offset.is_some() {
            let limit = stmt.limit.and_then(|expr| self.evaluate_constant(&expr).ok())
                .and_then(|val| if let Value::Integer(i) = val { Some(i as usize) } else { None })
                .unwrap_or(usize::MAX);
            
            let offset = stmt.offset.and_then(|expr| self.evaluate_constant(&expr).ok())
                .and_then(|val| if let Value::Integer(i) = val { Some(i as usize) } else { None })
                .unwrap_or(0);
            
            plan = LogicalPlan::Limit {
                input: Box::new(plan),
                limit,
                offset,
            };
        }
        
        // TODO: Apply ORDER BY (requires sort operator)
        
        Ok(plan)
    }
    
    fn plan_delete(&self, stmt: DeleteStmt) -> Result<LogicalPlan, SqlError> {
        let schema = self.catalog.get_table(&stmt.table_name)
            .ok_or_else(|| SqlError::TableNotFound(stmt.table_name.clone()))?
            .clone();
        
        Ok(LogicalPlan::Delete {
            table_name: stmt.table_name,
            filter: stmt.where_clause,
            schema,
        })
    }
    
    fn resolve_select_items(
        &self,
        items: &[SelectItem],
        schema: &TableSchema,
    ) -> Result<Vec<String>, SqlError> {
        let mut columns = Vec::new();
        
        for item in items {
            match item {
                SelectItem::Wildcard => {
                    // Add all columns
                    columns.extend(schema.columns.iter().map(|c| c.name.clone()));
                }
                SelectItem::Expr { expr, alias } => {
                    // For now, only support column references
                    if let Expr::ColumnRef(col_name) = expr {
                        // Verify column exists
                        if !schema.columns.iter().any(|c| &c.name == col_name) {
                            return Err(SqlError::ColumnNotFound(col_name.clone()));
                        }
                        columns.push(alias.as_ref().unwrap_or(col_name).clone());
                    } else {
                        // TODO: Support expressions
                        return Err(SqlError::PlannerError(
                            "Expressions in SELECT not yet supported".into()
                        ));
                    }
                }
            }
        }
        
        Ok(columns)
    }
    
    fn evaluate_constant(&self, expr: &Expr) -> Result<Value, SqlError> {
        match expr {
            Expr::Literal(val) => Ok(val.clone()),
            _ => Err(SqlError::PlannerError(
                "Non-constant expression in LIMIT/OFFSET".into()
            )),
        }
    }
}