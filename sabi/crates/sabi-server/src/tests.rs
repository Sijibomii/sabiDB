//! Unit tests for sabi-server

#[cfg(test)]
mod protocol_tests {
    use crate::protocol::*;
    use serde_json::json;

    #[test]
    fn test_message_envelope_serialization() {
        let envelope = MessageEnvelope {
            version: PROTOCOL_VERSION,
            message_type: MessageType::Query,
            request_id: Some("test-123".to_string()),
            payload: json!({"sql": "SELECT 1"}),
        };

        let json = serde_json::to_string(&envelope).expect("Should serialize");
        assert!(json.contains("QUERY"));
        assert!(json.contains("test-123"));
    }

    #[test]
    fn test_message_envelope_deserialization() {
        let json = r#"{"v":1,"type":"QUERY","request_id":"abc","payload":{"sql":"SELECT 1"}}"#;
        let envelope: MessageEnvelope = serde_json::from_str(json).expect("Should deserialize");

        assert_eq!(envelope.version, 1);
        assert!(matches!(envelope.message_type, MessageType::Query));
        assert_eq!(envelope.request_id, Some("abc".to_string()));
    }

    #[test]
    fn test_query_request() {
        let req = QueryRequest {
            sql: "SELECT * FROM users".to_string(),
            args: vec![json!(1), json!("test")],
            at_tx: Some(100),
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        let parsed: QueryRequest = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(parsed.sql, "SELECT * FROM users");
        assert_eq!(parsed.args.len(), 2);
        assert_eq!(parsed.at_tx, Some(100));
    }

    #[test]
    fn test_query_response() {
        let resp = QueryResponse {
            rows: vec![json!({"id": 1, "name": "Alice"})],
            read_tx: 100,
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("read_tx"));
        assert!(json.contains("Alice"));
    }

    #[test]
    fn test_mutation_request() {
        let req = MutationRequest {
            sql: "INSERT INTO users(name) VALUES(?)".to_string(),
            args: vec![json!("Bob")],
            client_tx: 99,
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        assert!(json.contains("client_tx"));
        assert!(json.contains("Bob"));
    }

    #[test]
    fn test_mutation_response() {
        let resp = MutationResponse {
            tx_id: 100,
            affected_rows: 1,
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        let parsed: MutationResponse = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(parsed.tx_id, 100);
        assert_eq!(parsed.affected_rows, 1);
    }

    #[test]
    fn test_function_request() {
        let req = FunctionRequest {
            name: "sendEmail".to_string(),
            args: json!({"to": "user@example.com", "subject": "Hello"}),
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        assert!(json.contains("sendEmail"));
    }

    #[test]
    fn test_function_response() {
        let resp = FunctionResponse {
            tx_id: 101,
            result: json!({"success": true}),
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        let parsed: FunctionResponse = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(parsed.tx_id, 101);
    }

    #[test]
    fn test_subscribe_request() {
        let req = SubscribeRequest {
            query: "SELECT * FROM events WHERE active = true".to_string(),
            args: vec![],
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        assert!(json.contains("events"));
    }

    #[test]
    fn test_subscribe_response() {
        let resp = SubscribeResponse {
            subscription_id: "sub-12345".to_string(),
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("subscription_id"));
        assert!(json.contains("sub-12345"));
    }

    #[test]
    fn test_update_message() {
        let msg = UpdateMessage {
            subscription_id: "sub-123".to_string(),
            tx_id: 200,
            rows: vec![json!({"id": 1, "status": "changed"})],
        };

        let json = serde_json::to_string(&msg).expect("Should serialize");
        assert!(json.contains("subscription_id"));
        assert!(json.contains("tx_id"));
    }

    #[test]
    fn test_unsubscribe_request() {
        let req = UnsubscribeRequest {
            subscription_id: "sub-123".to_string(),
        };

        let json = serde_json::to_string(&req).expect("Should serialize");
        let parsed: UnsubscribeRequest = serde_json::from_str(&json).expect("Should deserialize");

        assert_eq!(parsed.subscription_id, "sub-123");
    }

    #[test]
    fn test_hello_message() {
        let msg = HelloMessage {
            last_seen_tx: Some(500),
        };

        let json = serde_json::to_string(&msg).expect("Should serialize");
        assert!(json.contains("last_seen_tx"));
    }

    #[test]
    fn test_hello_message_no_tx() {
        let msg = HelloMessage {
            last_seen_tx: None,
        };

        let json = serde_json::to_string(&msg).expect("Should serialize");
        let parsed: HelloMessage = serde_json::from_str(&json).expect("Should deserialize");

        assert!(parsed.last_seen_tx.is_none());
    }

    #[test]
    fn test_welcome_message() {
        let msg = WelcomeMessage {
            current_tx: 1000,
        };

        let json = serde_json::to_string(&msg).expect("Should serialize");
        assert!(json.contains("current_tx"));
        assert!(json.contains("1000"));
    }

    #[test]
    fn test_error_codes() {
        let codes = vec![
            (ErrorCode::SerializationFailure, "SERIALIZATION_FAILURE"),
            (ErrorCode::SyntaxError, "SYNTAX_ERROR"),
            (ErrorCode::Internal, "INTERNAL"),
            (ErrorCode::NotFound, "NOT_FOUND"),
            (ErrorCode::Unauthorized, "UNAUTHORIZED"),
            (ErrorCode::RateLimited, "RATE_LIMITED"),
        ];

        for (code, expected) in codes {
            let json = serde_json::to_string(&code).expect("Should serialize");
            assert!(json.contains(expected), "Expected {} to contain {}", json, expected);
        }
    }

    #[test]
    fn test_error_response() {
        let resp = ErrorResponse {
            code: ErrorCode::SyntaxError,
            message: "Invalid SQL syntax".to_string(),
            details: Some(json!({"position": 42})),
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        assert!(json.contains("SYNTAX_ERROR"));
        assert!(json.contains("Invalid SQL syntax"));
        assert!(json.contains("position"));
    }

    #[test]
    fn test_error_response_no_details() {
        let resp = ErrorResponse {
            code: ErrorCode::Internal,
            message: "Something went wrong".to_string(),
            details: None,
        };

        let json = serde_json::to_string(&resp).expect("Should serialize");
        // details should be omitted when None
        assert!(!json.contains("details") || json.contains("null"));
    }

    #[test]
    fn test_error_response_display() {
        let resp = ErrorResponse {
            code: ErrorCode::SyntaxError,
            message: "Parse error".to_string(),
            details: None,
        };

        let display = format!("{}", resp);
        assert!(display.contains("SYNTAX_ERROR"));
        assert!(display.contains("Parse error"));
    }

    #[test]
    fn test_error_code_display() {
        assert_eq!(format!("{}", ErrorCode::SyntaxError), "SYNTAX_ERROR");
        assert_eq!(format!("{}", ErrorCode::Internal), "INTERNAL");
    }

    #[test]
    fn test_message_types() {
        let types = vec![
            MessageType::Query,
            MessageType::Mutation,
            MessageType::Subscribe,
            MessageType::Unsubscribe,
            MessageType::Update,
            MessageType::Error,
            MessageType::Ack,
            MessageType::Hello,
            MessageType::Welcome,
        ];

        for msg_type in types {
            let json = serde_json::to_string(&msg_type).expect("Should serialize");
            let parsed: MessageType = serde_json::from_str(&json).expect("Should deserialize");

            // Round-trip should work
            assert_eq!(
                serde_json::to_string(&msg_type).unwrap(),
                serde_json::to_string(&parsed).unwrap()
            );
        }
    }

    #[test]
    fn test_server_metrics() {
        let metrics = ServerMetrics {
            connections: 10,
            subscriptions: 25,
            queries_per_second: 100.5,
            mutations_per_second: 50.2,
            current_tx_id: 1000,
            uptime_seconds: 3600,
        };

        let json = serde_json::to_string(&metrics).expect("Should serialize");
        assert!(json.contains("connections"));
        assert!(json.contains("queries_per_second"));
        assert!(json.contains("uptime_seconds"));
    }

    #[test]
    fn test_protocol_version() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }
}

#[cfg(test)]
mod client_tracker_tests {
    use crate::client_tracker::{ClientTracker};
    use uuid::Uuid;
    use std::time::Duration;
    use std::thread;

    #[test]
    fn test_add_client() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);

        assert_eq!(tracker.client_count(), 1);
        assert!(tracker.get_client(client_id).is_some());
    }

    #[test]
    fn test_remove_client() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        assert_eq!(tracker.client_count(), 1);

        tracker.remove_client(client_id);
        assert_eq!(tracker.client_count(), 0);
        assert!(tracker.get_client(client_id).is_none());
    }

    #[test]
    fn test_multiple_clients() {
        let mut tracker = ClientTracker::new();
        let client1 = Uuid::now_v7();
        let client2 = Uuid::now_v7();
        let client3 = Uuid::now_v7();

        tracker.add_client(client1);
        tracker.add_client(client2);
        tracker.add_client(client3);

        assert_eq!(tracker.client_count(), 3);
    }

    #[test]
    fn test_update_last_seen_tx() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.update_last_seen_tx(client_id, Some(100));

        let state = tracker.get_client(client_id).expect("Client should exist");
        assert_eq!(state.last_seen_tx, Some(100));
    }

    #[test]
    fn test_add_subscription() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.add_subscription(
            client_id,
            "sub-123",
            "SELECT * FROM users",
            vec![],
        );

        assert_eq!(tracker.subscription_count(), 1);

        let state = tracker.get_client(client_id).expect("Client should exist");
        assert_eq!(state.subscriptions.len(), 1);
        assert!(state.subscriptions.contains_key("sub-123"));
    }

    #[test]
    fn test_multiple_subscriptions() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.add_subscription(client_id, "sub-1", "SELECT 1", vec![]);
        tracker.add_subscription(client_id, "sub-2", "SELECT 2", vec![]);
        tracker.add_subscription(client_id, "sub-3", "SELECT 3", vec![]);

        assert_eq!(tracker.subscription_count(), 3);

        let state = tracker.get_client(client_id).expect("Client should exist");
        assert_eq!(state.subscriptions.len(), 3);
    }

    #[test]
    fn test_remove_subscription() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.add_subscription(client_id, "sub-123", "SELECT * FROM users", vec![]);

        assert_eq!(tracker.subscription_count(), 1);

        tracker.remove_subscription(client_id, "sub-123");

        assert_eq!(tracker.subscription_count(), 0);

        let state = tracker.get_client(client_id).expect("Client should exist");
        assert!(state.subscriptions.is_empty());
    }

    #[test]
    fn test_remove_client_removes_subscriptions() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.add_subscription(client_id, "sub-1", "SELECT 1", vec![]);
        tracker.add_subscription(client_id, "sub-2", "SELECT 2", vec![]);

        assert_eq!(tracker.subscription_count(), 2);

        tracker.remove_client(client_id);

        assert_eq!(tracker.subscription_count(), 0);
        assert_eq!(tracker.client_count(), 0);
    }

    #[test]
    fn test_shared_subscription_cleanup() {
        let mut tracker = ClientTracker::new();
        let client1 = Uuid::now_v7();
        let client2 = Uuid::now_v7();

        tracker.add_client(client1);
        tracker.add_client(client2);

        // Both clients subscribe to the same subscription ID
        tracker.add_subscription(client1, "shared-sub", "SELECT 1", vec![]);
        tracker.add_subscription(client2, "shared-sub", "SELECT 1", vec![]);

        // Subscription should still be tracked for both
        assert_eq!(tracker.subscription_count(), 1); // Same subscription ID

        // Remove first client
        tracker.remove_client(client1);

        // Subscription should still exist for second client
        assert_eq!(tracker.subscription_count(), 1);

        // Remove second client
        tracker.remove_client(client2);

        // Now subscription should be gone
        assert_eq!(tracker.subscription_count(), 0);
    }

    #[test]
    fn test_get_all_clients() {
        let mut tracker = ClientTracker::new();
        let client1 = Uuid::now_v7();
        let client2 = Uuid::now_v7();

        tracker.add_client(client1);
        tracker.add_client(client2);

        let clients = tracker.get_all_clients();
        assert_eq!(clients.len(), 2);

        let client_ids: Vec<Uuid> = clients.iter().map(|c| c.client_id).collect();
        assert!(client_ids.contains(&client1));
        assert!(client_ids.contains(&client2));
    }

    #[test]
    fn test_get_clients_for_key() {
        let mut tracker = ClientTracker::new();
        let client1 = Uuid::now_v7();
        let client2 = Uuid::now_v7();

        tracker.add_client(client1);
        tracker.add_client(client2);

        let clients = tracker.get_clients_for_key("users:1");
        assert_eq!(clients.len(), 2); // Returns all clients (simplified impl)
    }

    #[test]
    fn test_cleanup_stale_clients() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);

        // Wait a bit to make the client "stale"
        thread::sleep(Duration::from_millis(100));

        // Cleanup with a very short timeout
        let stale = tracker.cleanup_stale_clients(Duration::from_millis(50));

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0], client_id);
        assert_eq!(tracker.client_count(), 0);
    }

    #[test]
    fn test_cleanup_keeps_active_clients() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);

        // Cleanup with a long timeout - client should NOT be stale
        let stale = tracker.cleanup_stale_clients(Duration::from_secs(60));

        assert!(stale.is_empty());
        assert_eq!(tracker.client_count(), 1);
    }

    #[test]
    fn test_client_state_fields() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);

        let state = tracker.get_client(client_id).expect("Client should exist");

        assert_eq!(state.client_id, client_id);
        assert!(state.last_seen_tx.is_none());
        assert!(state.subscriptions.is_empty());
    }

    #[test]
    fn test_subscription_info_fields() {
        let mut tracker = ClientTracker::new();
        let client_id = Uuid::now_v7();

        tracker.add_client(client_id);
        tracker.add_subscription(
            client_id,
            "sub-test",
            "SELECT * FROM events",
            vec![serde_json::json!(true)],
        );

        let state = tracker.get_client(client_id).expect("Client should exist");
        let sub_info = state.subscriptions.get("sub-test").expect("Sub should exist");

        assert_eq!(sub_info.query, "SELECT * FROM events");
        assert_eq!(sub_info.args.len(), 1);
        assert!(sub_info.last_update_tx.is_none());
    }

    #[test]
    fn test_update_nonexistent_client() {
        let mut tracker = ClientTracker::new();
        let fake_client = Uuid::now_v7();

        // Should not panic, just no-op
        tracker.update_last_seen_tx(fake_client, Some(100));

        assert_eq!(tracker.client_count(), 0);
    }

    #[test]
    fn test_remove_nonexistent_client() {
        let mut tracker = ClientTracker::new();
        let fake_client = Uuid::now_v7();

        // Should not panic, just no-op
        tracker.remove_client(fake_client);

        assert_eq!(tracker.client_count(), 0);
    }
}
