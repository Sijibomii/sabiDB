//! Unit tests for sabi-core

#[cfg(test)]
mod tx_tests {
    use crate::tx::TxId;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_txid_creation() {
        let tx = TxId::new();
        assert!(!tx.is_nil());
    }

    #[test]
    fn test_txid_nil() {
        let tx = TxId::nil();
        assert!(tx.is_nil());
    }

    #[test]
    fn test_txid_ordering() {
        let tx1 = TxId::new();
        thread::sleep(Duration::from_millis(10));
        let tx2 = TxId::new();

        // tx2 should be greater than tx1 (created later)
        assert!(tx2 > tx1);
        assert!(tx1 < tx2);
    }

    #[test]
    fn test_txid_from_timestamp() {
        let tx1 = TxId::from_timestamp(1000, 0);
        let tx2 = TxId::from_timestamp(2000, 0);

        assert!(tx2 > tx1);
    }

    #[test]
    fn test_txid_equality() {
        let tx = TxId::new();
        let tx_clone = tx;

        assert_eq!(tx, tx_clone);
    }

    #[test]
    fn test_txid_timestamp() {
        let tx = TxId::new();
        let ts = tx.timestamp();

        // Should have a valid timestamp
        assert!(ts.is_some());
    }

    #[test]
    fn test_txid_display() {
        let tx = TxId::new();
        let display = format!("{}", tx);

        // UUID format should be 36 characters
        assert_eq!(display.len(), 36);
    }

    #[test]
    fn test_txid_from_str() {
        let tx = TxId::new();
        let s = format!("{}", tx);
        let parsed: TxId = s.parse().expect("Should parse valid UUID");

        assert_eq!(tx, parsed);
    }

    #[test]
    fn test_txid_hash() {
        use std::collections::HashSet;

        let tx1 = TxId::new();
        let tx2 = TxId::new();

        let mut set = HashSet::new();
        set.insert(tx1);
        set.insert(tx2);

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn test_txid_serialization() {
        let tx = TxId::new();
        let json = serde_json::to_string(&tx).expect("Should serialize");
        let deserialized: TxId = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(tx, deserialized);
    }
}

#[cfg(test)]
mod timestamp_tests {
    use crate::timestamp::Timestamp;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_timestamp_now() {
        let ts = Timestamp::now();
        assert!(ts.0 > 0);
    }

    #[test]
    fn test_timestamp_ordering() {
        let ts1 = Timestamp::now();
        thread::sleep(Duration::from_millis(10));
        let ts2 = Timestamp::now();

        assert!(ts2 > ts1);
    }

    #[test]
    fn test_timestamp_display() {
        let ts = Timestamp::now();
        let display = format!("{}", ts);

        // Should be a number string
        assert!(!display.is_empty());
        assert!(display.parse::<u64>().is_ok());
    }

    #[test]
    fn test_timestamp_equality() {
        let ts = Timestamp(12345);
        let ts2 = Timestamp(12345);

        assert_eq!(ts, ts2);
    }
}

#[cfg(test)]
mod error_tests {
    use crate::error::DbError;
    use std::io;

    #[test]
    fn test_syntax_error_display() {
        let err = DbError::SyntaxError("invalid query".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Syntax Error"));
        assert!(display.contains("invalid query"));
    }

    #[test]
    fn test_serialization_failure_display() {
        let err = DbError::SerializationFailure("conflict".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Serialization Failure"));
    }

    #[test]
    fn test_internal_error_display() {
        let err = DbError::Internal("internal issue".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Internal Error"));
    }

    #[test]
    fn test_corruption_error_display() {
        let err = DbError::Corruption("data corrupted".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Corruption"));
    }

    #[test]
    fn test_not_found_error_display() {
        let err = DbError::NotFound("key not found".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Not Found"));
    }

    #[test]
    fn test_invalid_transaction_display() {
        let err = DbError::InvalidTransaction("tx aborted".to_string());
        let display = format!("{}", err);

        assert!(display.contains("Invalid Transaction"));
    }

    #[test]
    fn test_io_error_conversion() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let db_err: DbError = io_err.into();

        match db_err {
            DbError::Io(_) => {}
            _ => panic!("Expected Io variant"),
        }
    }

    #[test]
    fn test_error_is_error_trait() {
        let err = DbError::Internal("test".to_string());

        // Verify it implements std::error::Error
        let _: &dyn std::error::Error = &err;
    }
}

#[cfg(test)]
mod protocol_tests {
    use crate::protocol::{Request, Response, ErrorPayload, ErrorCode};
    use crate::tx::TxId;
    use serde_json::json;

    #[test]
    fn test_query_request_serialization() {
        let tx = TxId::new();
        let req = Request::Query {
            request_id: tx,
            sql: "SELECT * FROM users".to_string(),
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        assert!(json.contains("Query"));
        assert!(json.contains("SELECT * FROM users"));
    }

    #[test]
    fn test_query_request_deserialization() {
        let tx = TxId::new();
        let json = format!(
            r#"{{"type":"Query","request_id":"{}","sql":"SELECT 1"}}"#,
            tx
        );

        let req: Request = serde_json::from_str(&json).expect("Should deserialize");
        match req {
            Request::Query { sql, .. } => assert_eq!(sql, "SELECT 1"),
            _ => panic!("Expected Query variant"),
        }
    }

    #[test]
    fn test_subscribe_request_serialization() {
        let tx = TxId::new();
        let req = Request::Subscribe {
            request_id: tx,
            sql: "SELECT * FROM events".to_string(),
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        assert!(json.contains("Subscribe"));
    }

    #[test]
    fn test_ok_response_serialization() {
        let tx = TxId::new();
        let resp = Response::Ok {
            request_id: tx,
            payload: json!({"rows": []}),
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("Ok"));
    }

    #[test]
    fn test_error_response_serialization() {
        let tx = TxId::new();
        let resp = Response::Error {
            request_id: tx,
            payload: ErrorPayload {
                code: ErrorCode::SyntaxError,
                message: "Invalid SQL".to_string(),
            },
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("Error"));
        assert!(json.contains("SYNTAX_ERROR"));
    }

    #[test]
    fn test_update_response_serialization() {
        let tx = TxId::new();
        let resp = Response::Update {
            subscription_id: tx,
            payload: json!({"changed": true}),
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("Update"));
    }

    #[test]
    fn test_error_code_serialization() {
        let codes = vec![
            ErrorCode::SyntaxError,
            ErrorCode::SerializationFailure,
            ErrorCode::Internal,
        ];

        for code in codes {
            let json = serde_json::to_string(&code).expect("Should serialize");
            let deserialized: ErrorCode = serde_json::from_str(&json).expect("Should deserialize");

            // Verify round-trip
            assert_eq!(
                serde_json::to_string(&code).unwrap(),
                serde_json::to_string(&deserialized).unwrap()
            );
        }
    }
}
