use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use sabi_core::TxId;
use sabi_storage::engine::{StorageEngine};
use sabi_storage::mvcc::Transaction;

use crate::parser::{BinaryOperator, Expr, UnaryOperator};
use crate::planner::{LogicalPlan, Catalog};
use crate::types::{DataType, TableSchema, Value};
use crate::error::SqlError;

/// Query execution result
#[derive(Debug)]
pub enum QueryResult {
    CreateTable,
    Insert(usize),
    Select {
        columns: Vec<String>,
        rows: Vec<Vec<Value>>,
    },
    Delete(usize),
    BeginTransaction(TxId),
    Commit,
    Rollback,
}

/// Query executor - executes logical plans against storage
pub struct QueryExecutor {
    storage: Arc<Mutex<StorageEngine>>,
    catalog: Catalog,
    current_transaction: Option<Transaction>,
    auto_commit: bool,
}

impl QueryExecutor {
    pub fn new(storage: Arc<Mutex<StorageEngine>>, catalog: Catalog) -> Self {
        Self {
            storage,
            catalog,
            current_transaction: None,
            auto_commit: true,
        }
    }
    
    /// Execute a logical plan
    pub fn execute(&mut self, plan: LogicalPlan) -> Result<QueryResult, SqlError> {
        match plan {
            LogicalPlan::CreateTable { schema, if_not_exists } => {
                self.execute_create_table(schema, if_not_exists)
            }
            LogicalPlan::Insert { table_name, columns, values, schema } => {
                self.execute_insert(table_name, columns, values, schema)
            }
            LogicalPlan::Scan { table_name, columns, filter, schema } => {
                self.execute_scan(table_name, columns, filter, schema)
            }
            LogicalPlan::Filter { input, predicate } => {
                self.execute_filter(*input, predicate)
            }
            LogicalPlan::Limit { input, limit, offset } => {
                self.execute_limit(*input, limit, offset)
            }
            LogicalPlan::Delete { table_name, filter, schema } => {
                self.execute_delete(table_name, filter, schema)
            }
            LogicalPlan::BeginTransaction => self.execute_begin(),
            LogicalPlan::Commit => self.execute_commit(),
            LogicalPlan::Rollback => self.execute_rollback(),
            _ => Err(SqlError::ExecutionError("Unsupported plan node".into())),
        }
    }
    
    fn execute_create_table(
        &mut self,
        schema: TableSchema,
        if_not_exists: bool,
    ) -> Result<QueryResult, SqlError> {
        let mut storage = self.storage.lock().unwrap();
        
        // Check if table exists
        if if_not_exists && storage.tables.contains_key(&schema.name) {
            return Ok(QueryResult::CreateTable);
        }
        
        // Create table in storage
        storage.tables.insert(schema.name.clone(), sabi_storage::mvcc::MvccTable::new());
        
        // Add to catalog
        self.catalog.add_table(schema);
        
        Ok(QueryResult::CreateTable)
    }
    
    fn execute_insert(
        &mut self,
        table_name: String,
        columns: Option<Vec<String>>,
        values: Vec<Vec<Expr>>,
        schema: TableSchema,
    ) -> Result<QueryResult, SqlError> {
        let tx = self.begin_transaction()?;
        let mut storage = self.storage.lock().unwrap();
        
        let mut rows_inserted = 0;
        
        for row_values in values {
            // Build key from primary key columns
            let key = self.build_row_key(&schema, &columns, &row_values)?;
            
            // Serialize row
            let row_data = self.serialize_row(&schema, &columns, row_values)?;
            
            // Insert into storage
            storage.insert(&table_name, key, row_data, &tx)
                .map_err(|e| SqlError::Storage(e))?;
            
            rows_inserted += 1;
        }
        
        if self.auto_commit {
            storage.commit(tx.tx_id)
                .map_err(|e| SqlError::Storage(e))?;
            self.current_transaction = None;
        }
        
        Ok(QueryResult::Insert(rows_inserted))
    }
    
    fn execute_scan(
        &mut self,
        table_name: String,
        columns: Vec<String>,
        filter: Option<Expr>,
        schema: TableSchema,
    ) -> Result<QueryResult, SqlError> {
        let tx = self.begin_transaction()?;
        let mut storage = self.storage.lock().unwrap();
        
        // For now, do full table scan
        // TODO: Use B-Tree index when available
        let mut rows = Vec::new();
        
        // This is a placeholder - you'll need to implement table iteration
        // based on your storage engine's capabilities
        let all_rows = storage.range_scan(&table_name, &[], &[], &tx)
            .map_err(|e| SqlError::Storage(e))?;
        
        for (_, value_bytes) in all_rows {
            // Deserialize row
            let row = self.deserialize_row(&schema, &value_bytes)?;
            
            // Filter columns
            let selected_row: Vec<Value> = if columns.is_empty() {
                row.iter().cloned().collect()
            } else {
                columns.iter()
                    .filter_map(|col| schema.columns.iter()
                        .position(|c| &c.name == col)
                        .map(|idx| row[idx].clone()))
                    .collect()
            };
            
            // Apply filter if specified
            let include = if let Some(ref filter_expr) = filter {
                self.evaluate_filter(&selected_row, &columns, filter_expr)?
            } else {
                true
            };
            
            if include {
                rows.push(selected_row);
            }
        }
        
        Ok(QueryResult::Select { columns, rows })
    }
    
    fn execute_filter(
        &mut self,
        input: LogicalPlan,
        predicate: Expr,
    ) -> Result<QueryResult, SqlError> {
        // Execute input plan
        let result = self.execute(input)?;
        
        match result {
            QueryResult::Select { columns, mut rows } => {
                // Apply filter to each row
                rows.retain(|row| {
                    self.evaluate_filter(row, &columns, &predicate)
                        .unwrap_or(false)
                });
                
                Ok(QueryResult::Select { columns, rows })
            }
            _ => Err(SqlError::ExecutionError("Filter requires SELECT input".into())),
        }
    }
    
    fn execute_limit(
        &mut self,
        input: LogicalPlan,
        limit: usize,
        offset: usize,
    ) -> Result<QueryResult, SqlError> {
        let result = self.execute(input)?;
        
        match result {
            QueryResult::Select { columns, mut rows } => {
                // Apply offset and limit
                if offset >= rows.len() {
                    rows.clear();
                } else {
                    let start = offset;
                    let end = (offset + limit).min(rows.len());
                    rows = rows[start..end].to_vec();
                }
                
                Ok(QueryResult::Select { columns, rows })
            }
            _ => Err(SqlError::ExecutionError("Limit requires SELECT input".into())),
        }
    }
    
    fn execute_delete(
        &mut self,
        table_name: String,
        filter: Option<Expr>,
        schema: TableSchema,
    ) -> Result<QueryResult, SqlError> {
        let tx = self.begin_transaction()?;
        let mut storage = self.storage.lock().unwrap();
        
        // For now, scan and delete matching rows
        let mut rows_deleted = 0;
        let all_rows = storage.range_scan(&table_name, &[], &[], &tx)
            .map_err(|e| SqlError::Storage(e))?;
        
        for (key, value_bytes) in all_rows {
            let row = self.deserialize_row(&schema, &value_bytes)?;
            
            let should_delete = if let Some(ref filter_expr) = filter {
                // Evaluate filter against all columns
                let all_columns: Vec<String> = schema.columns.iter()
                    .map(|c| c.name.clone())
                    .collect();
                self.evaluate_filter(&row, &all_columns, filter_expr)?
            } else {
                true // DELETE without WHERE deletes all rows
            };
            
            if should_delete {
                storage.delete(&table_name, &key, &tx)
                    .map_err(|e| SqlError::Storage(e))?;
                rows_deleted += 1;
            }
        }
        
        if self.auto_commit {
            storage.commit(tx.tx_id)
                .map_err(|e| SqlError::Storage(e))?;
            self.current_transaction = None;
        }
        
        Ok(QueryResult::Delete(rows_deleted))
    }
    
    fn execute_begin(&mut self) -> Result<QueryResult, SqlError> {
        if self.current_transaction.is_some() {
            return Err(SqlError::ExecutionError(
                "Transaction already in progress".into()
            ));
        }
        
        let storage = self.storage.lock().unwrap();
        let tx = storage.begin(false); // Read-write transaction
        self.current_transaction = Some(tx.clone());
        self.auto_commit = false;
        
        Ok(QueryResult::BeginTransaction(tx.tx_id))
    }
    
    fn execute_commit(&mut self) -> Result<QueryResult, SqlError> {
        let tx = self.current_transaction.take()
            .ok_or_else(|| SqlError::ExecutionError("No transaction to commit".into()))?;
        
        let storage = self.storage.lock().unwrap();
        storage.commit(tx.tx_id)
            .map_err(|e| SqlError::Storage(e))?;
        
        self.auto_commit = true;
        Ok(QueryResult::Commit)
    }
    
    fn execute_rollback(&mut self) -> Result<QueryResult, SqlError> {
        let tx = self.current_transaction.take()
            .ok_or_else(|| SqlError::ExecutionError("No transaction to rollback".into()))?;
        
        let storage = self.storage.lock().unwrap();
        storage.abort(tx.tx_id)
            .map_err(|e| SqlError::Storage(e))?;
        
        self.auto_commit = true;
        Ok(QueryResult::Rollback)
    }
    
    // Helper methods
    
    fn begin_transaction(&self) -> Result<Transaction, SqlError> {
        if let Some(ref tx) = self.current_transaction {
            Ok(tx.clone())
        } else {
            let storage = self.storage.lock().unwrap();
            Ok(storage.begin(true)) // Read-only for queries
        }
    }
    
    fn build_row_key(
        &self,
        schema: &TableSchema,
        columns: &Option<Vec<String>>,
        values: &[Expr],
    ) -> Result<Vec<u8>, SqlError> {
        // Use primary key columns for key
        if let Some(ref pk_cols) = schema.primary_key {
            let mut key_parts = Vec::new();
            
            for pk_col in pk_cols {
                let idx = if let Some(cols) = columns {
                    cols.iter().position(|c| c == pk_col)
                        .ok_or_else(|| SqlError::ColumnNotFound(pk_col.clone()))?
                } else {
                    schema.columns.iter()
                        .position(|c| &c.name == pk_col)
                        .ok_or_else(|| SqlError::ColumnNotFound(pk_col.clone()))?
                };
                
                if idx >= values.len() {
                    return Err(SqlError::ExecutionError(
                        format!("Primary key column '{}' not provided", pk_col)
                    ));
                }
                
                // Evaluate expression (should be constant for INSERT)
                let value = self.evaluate_expression(&values[idx], &HashMap::new())?;
                key_parts.push(value.to_bytes());
            }
            
            // Concatenate key parts with separator
            let mut key = Vec::new();
            for part in key_parts {
                key.extend_from_slice(&(part.len() as u32).to_le_bytes());
                key.extend_from_slice(&part);
            }
            
            Ok(key)
        } else {
            // No primary key - use all columns
            let mut row_data = Vec::new();
            for value in values {
                let val = self.evaluate_expression(value, &HashMap::new())?;
                row_data.extend_from_slice(&val.to_bytes());
            }
            Ok(row_data)
        }
    }
    
    fn serialize_row(
        &self,
        schema: &TableSchema,
        columns: &Option<Vec<String>>,
        values: Vec<Expr>,
    ) -> Result<Vec<u8>, SqlError> {
        let mut row_data = Vec::new();
        
        // If columns specified, use that order; otherwise use schema order
        let col_names: Vec<&String> = if let Some(cols) = columns {
            cols.iter().collect()
        } else {
            schema.columns.iter().map(|c| &c.name).collect()
        };
        
        for (i, col_name) in col_names.iter().enumerate() {
            // Find column in schema
    
            let col = schema.columns.iter()
                .find(|c| &c.name == *col_name)
                .ok_or_else(|| SqlError::ColumnNotFound(col_name.to_string()))?;
            
            // Evaluate value
            let value = self.evaluate_expression(&values[i], &HashMap::new())?;
            
            // Type check
            if let Some(expected_type) = value.data_type() {
                match (&expected_type, &col.data_type) {
                    (DataType::Integer, DataType::Integer) |
                    (DataType::Text, DataType::Text) |
                    (DataType::Boolean, DataType::Boolean) => {
                        // Types match
                    }
                    _ => {
                        return Err(SqlError::TypeError {
                            expected: col.data_type.to_string(),
                            actual: expected_type.to_string(),
                        });
                    }
                }
            }
            
            // Serialize value
            row_data.extend_from_slice(&value.to_bytes());
        }
        
        Ok(row_data)
    }
    
    fn deserialize_row(
        &self,
        schema: &TableSchema,
        bytes: &[u8],
    ) -> Result<Vec<Value>, SqlError> {
        let mut values = Vec::new();
        let mut cursor = 0;
        
        for _ in 0..schema.columns.len() {
            if cursor >= bytes.len() {
                break;
            }
            
            // Read value tag
            let tag = bytes[cursor];
            cursor += 1;
            
            let value = match tag {
                0 => Value::Null,
                1 => {
                    if cursor + 8 > bytes.len() {
                        return Err(SqlError::Internal("Row data truncated".into()));
                    }
                    let int_bytes: [u8; 8] = bytes[cursor..cursor + 8].try_into()
                        .map_err(|_| SqlError::Internal("Invalid integer bytes".into()))?;
                    cursor += 8;
                    Value::Integer(i64::from_le_bytes(int_bytes))
                }
                2 => {
                    if cursor + 4 > bytes.len() {
                        return Err(SqlError::Internal("Row data truncated".into()));
                    }
                    let len_bytes: [u8; 4] = bytes[cursor..cursor + 4].try_into()
                        .map_err(|_| SqlError::Internal("Invalid length bytes".into()))?;
                    let len = u32::from_le_bytes(len_bytes) as usize;
                    cursor += 4;
                    
                    if cursor + len > bytes.len() {
                        return Err(SqlError::Internal("Text data truncated".into()));
                    }
                    let text = String::from_utf8(bytes[cursor..cursor + len].to_vec())
                        .map_err(|e| SqlError::Internal(format!("Invalid UTF-8: {}", e)))?;
                    cursor += len;
                    Value::Text(text)
                }
                3 => {
                    if cursor >= bytes.len() {
                        return Err(SqlError::Internal("Row data truncated".into()));
                    }
                    Value::Boolean(bytes[cursor] != 0)
                }
                _ => return Err(SqlError::Internal("Unknown value type tag".into())),
            };
            
            values.push(value);
        }
        
        Ok(values)
    }
    
    fn evaluate_filter(
        &self,
        row: &[Value],
        columns: &[String],
        predicate: &Expr,
    ) -> Result<bool, SqlError> {
        // Create mapping from column names to values
        let mut context = HashMap::new();
        for (i, col) in columns.iter().enumerate() {
            if i < row.len() {
                context.insert(col.clone(), row[i].clone());
            }
        }
        
        self.evaluate_boolean(predicate, &context)
    }
    
    fn evaluate_expression(
        &self,
        expr: &Expr,
        context: &HashMap<String, Value>,
    ) -> Result<Value, SqlError> {
        match expr {
            Expr::Literal(val) => Ok(val.clone()),
            
            Expr::ColumnRef(col_name) => {
                context.get(col_name)
                    .cloned()
                    .ok_or_else(|| SqlError::ColumnNotFound(col_name.clone()))
            }
            
            Expr::BinaryExpr { left, op, right } => {
                let left_val = self.evaluate_expression(left, context)?;
                let right_val = self.evaluate_expression(right, context)?;
                
                match op {
                    BinaryOperator::Eq => Ok(Value::Boolean(left_val == right_val)),
                    BinaryOperator::Neq => Ok(Value::Boolean(left_val != right_val)),
                    BinaryOperator::Lt => self.compare_values(&left_val, &right_val, |a, b| a < b),
                    BinaryOperator::LtEq => self.compare_values(&left_val, &right_val, |a, b| a <= b),
                    BinaryOperator::Gt => self.compare_values(&left_val, &right_val, |a, b| a > b),
                    BinaryOperator::GtEq => self.compare_values(&left_val, &right_val, |a, b| a >= b),
                    BinaryOperator::And => {
                        let left_bool = self.value_to_bool(&left_val)?;
                        let right_bool = self.value_to_bool(&right_val)?;
                        Ok(Value::Boolean(left_bool && right_bool))
                    }
                    BinaryOperator::Or => {
                        let left_bool = self.value_to_bool(&left_val)?;
                        let right_bool = self.value_to_bool(&right_val)?;
                        Ok(Value::Boolean(left_bool || right_bool))
                    }
                    BinaryOperator::Add => self.arithmetic_op(&left_val, &right_val, |a, b| a + b),
                    BinaryOperator::Sub => self.arithmetic_op(&left_val, &right_val, |a, b| a - b),
                    BinaryOperator::Mul => self.arithmetic_op(&left_val, &right_val, |a, b| a * b),
                    BinaryOperator::Div => self.arithmetic_op(&left_val, &right_val, |a, b| a / b),
                    BinaryOperator::Like => self.like_op(&left_val, &right_val),
                }
            }
            
            Expr::UnaryExpr { op, expr } => {
                let val = self.evaluate_expression(expr, context)?;
                match op {
                    UnaryOperator::Not => {
                        let bool_val = self.value_to_bool(&val)?;
                        Ok(Value::Boolean(!bool_val))
                    }
                    UnaryOperator::Neg => {
                        if let Value::Integer(i) = val {
                            Ok(Value::Integer(-i))
                        } else {
                            Err(SqlError::TypeError {
                                expected: "INTEGER".into(),
                                actual: val.data_type().map(|t| t.to_string()).unwrap_or("NULL".into()),
                            })
                        }
                    }
                }
            }
            
            Expr::IsNull(expr) => {
                let val = self.evaluate_expression(expr, context)?;
                Ok(Value::Boolean(matches!(val, Value::Null)))
            }
            
            Expr::IsNotNull(expr) => {
                let val = self.evaluate_expression(expr, context)?;
                Ok(Value::Boolean(!matches!(val, Value::Null)))
            }
            
            Expr::Between { expr, low, high } => {
                let val = self.evaluate_expression(expr, context)?;
                let low_val = self.evaluate_expression(low, context)?;
                let high_val = self.evaluate_expression(high, context)?;
                
                let ge = self.compare_values(&val, &low_val, |a, b| a >= b)?;
                let le = self.compare_values(&val, &high_val, |a, b| a <= b)?;
                
                Ok(Value::Boolean(
                    self.value_to_bool(&ge)? && self.value_to_bool(&le)?
                ))
            }
            
            _ => Err(SqlError::ExecutionError("Expression not supported".into())),
        }
    }
    
    fn evaluate_boolean(
        &self,
        expr: &Expr,
        context: &HashMap<String, Value>,
    ) -> Result<bool, SqlError> {
        let value = self.evaluate_expression(expr, context)?;
        self.value_to_bool(&value)
    }
    
    fn value_to_bool(&self, value: &Value) -> Result<bool, SqlError> {
        match value {
            Value::Boolean(b) => Ok(*b),
            Value::Null => Ok(false),
            Value::Integer(i) => Ok(*i != 0),
            Value::Text(s) => Ok(!s.is_empty()),
        }
    }
    
    fn compare_values<F>(&self, left: &Value, right: &Value, comparator: F) -> Result<Value, SqlError>
        where
            F: Fn(&Value, &Value) -> bool,
    {
        // Check if values are comparable
        match (left, right) {
            (Value::Null, _) | (_, Value::Null) => {
                // In SQL, NULL compared with anything returns NULL (not false)
                Ok(Value::Null)
            }
            (Value::Integer(_), Value::Integer(_)) |
            (Value::Text(_), Value::Text(_)) |
            (Value::Boolean(_), Value::Boolean(_)) => {
                Ok(Value::Boolean(comparator(left, right)))
            }
            _ => Err(SqlError::TypeError {
                expected: format!("Values of comparable types"),
                actual: format!("{:?} and {:?}", left.data_type(), right.data_type()),
            }),
        }
    }
    
    fn arithmetic_op<F>(
        &self,
        left: &Value,
        right: &Value,
        op: F,
    ) -> Result<Value, SqlError>
    where
        F: FnOnce(i64, i64) -> i64,
    {
        match (left, right) {
            (Value::Integer(a), Value::Integer(b)) => Ok(Value::Integer(op(*a, *b))),
            _ => Err(SqlError::TypeError {
                expected: "INTEGER".into(),
                actual: format!("{:?} and {:?}", left.data_type(), right.data_type()),
            }),
        }
    }
    
    fn like_op(&self, left: &Value, right: &Value) -> Result<Value, SqlError> {
        match (left, right) {
            (Value::Text(text), Value::Text(pattern)) => {
                // Simple LIKE implementation (supports % and _)
                let regex_pattern = pattern
                    .replace("%", ".*")
                    .replace("_", ".");
                let regex = regex::Regex::new(&format!("^{}$", regex_pattern))
                    .map_err(|e| SqlError::ExecutionError(format!("Invalid LIKE pattern: {}", e)))?;
                
                Ok(Value::Boolean(regex.is_match(text)))
            }
            _ => Err(SqlError::TypeError {
                expected: "TEXT".into(),
                actual: format!("{:?} and {:?}", left.data_type(), right.data_type()),
            }),
        }
    }
}