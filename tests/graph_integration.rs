//! Comprehensive graph integration tests.
//!
//! Tests the full graph workflow: documents, edges, traversal, deletion.
//!
//! ## Traversal Target Syntax
//!
//! - `->follows` - Returns edges with only `id` field
//! - `->follows.*` - Returns edges with all fields (id, in, out, custom fields)
//! - `->follows.since` - Returns edges with only the `since` field
//! - `->follows->user` - Returns target node IDs
//! - `->follows->user.*` - Returns target nodes with all fields
//! - `->follows->user.name` - Returns target nodes with only `name` field
//!
//! ## Notes
//!
//! - Self-referential edges are correctly returned because edge deduplication is based on
//!   edge.id (for Edge targets) rather than target node

use std::sync::Arc;
use stellardb::Database;
use tempfile::TempDir;

fn setup() -> (TempDir, Arc<Database>) {
    let tmp = TempDir::new().unwrap();
    let storage = Arc::new(Database::open(tmp.path()).unwrap());
    (tmp, storage)
}

fn run(storage: &Arc<Database>, sql: &str) -> serde_json::Value {
    stellardb::query_json(storage.clone(), sql)
}

fn run_err(storage: &Arc<Database>, sql: &str) -> String {
    stellardb::try_run_sql!(storage.clone(), sql)
        .unwrap_err()
        .to_string()
}

// =============================================================================
// Full graph workflow test
// =============================================================================

#[test]
fn test_full_graph_workflow() {
    let (_tmp, storage) = setup();

    // 1. Create schema
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    // 2. Create documents
    run(&storage, "CREATE user:alice SET name = 'Alice', age = 30");
    run(&storage, "CREATE user:bob SET name = 'Bob', age = 25");
    run(&storage, "CREATE user:carol SET name = 'Carol', age = 28");
    run(
        &storage,
        "CREATE post:p1 SET title = 'Hello World', author = 'user:alice'",
    );
    run(
        &storage,
        "CREATE post:p2 SET title = 'Rust is great', author = 'user:bob'",
    );

    // 3. Create edges - social graph
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024",
    );
    run(
        &storage,
        "RELATE user:alice->follows->user:carol SET since = 2023",
    );
    run(
        &storage,
        "RELATE user:bob->follows->user:carol SET since = 2024",
    );
    run(
        &storage,
        "RELATE user:carol->follows->user:alice SET since = 2022",
    );

    // 4. Create edges - authorship
    run(&storage, "RELATE user:alice->wrote->post:p1");
    run(&storage, "RELATE user:bob->wrote->post:p2");

    // 5. Create edges - likes
    run(&storage, "RELATE user:alice->likes->post:p2");
    run(&storage, "RELATE user:bob->likes->post:p1");
    run(&storage, "RELATE user:carol->likes->post:p1");
    run(&storage, "RELATE user:carol->likes->post:p2");

    // 6. Query graph - who does Alice follow?
    let alice_follows = run(
        &storage,
        "SELECT ->follows->user.* AS following FROM user:alice",
    );
    let following = &alice_follows.as_array().unwrap()[0]["following"];
    let following_arr = following.as_array().unwrap();
    assert_eq!(following_arr.len(), 2, "Alice should follow 2 people");

    // 7. Query graph - who follows Carol? (incoming edges)
    let carol_followers = run(
        &storage,
        "SELECT <-follows<-user.* AS followers FROM user:carol",
    );
    let followers = &carol_followers.as_array().unwrap()[0]["followers"];
    let followers_arr = followers.as_array().unwrap();
    assert_eq!(followers_arr.len(), 2, "Carol should have 2 followers");

    // 8. Query graph - posts Alice wrote
    let alice_posts = run(&storage, "SELECT ->wrote->post.* AS posts FROM user:alice");
    let posts = &alice_posts.as_array().unwrap()[0]["posts"];
    let posts_arr = posts.as_array().unwrap();
    assert_eq!(posts_arr.len(), 1, "Alice should have 1 post");
    assert_eq!(posts_arr[0]["title"], "Hello World");

    // 9. Query graph - who likes post p1?
    let p1_likers = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p1");
    let likers = &p1_likers.as_array().unwrap()[0]["likers"];
    let likers_arr = likers.as_array().unwrap();
    assert_eq!(likers_arr.len(), 2, "Post p1 should have 2 likers");

    // 10. Delete specific edge
    run(&storage, "DELETE user:alice->follows->user:bob");

    // Verify deletion
    let after_delete = run(
        &storage,
        "SELECT ->follows->user.* AS following FROM user:alice",
    );
    let following_after = &after_delete.as_array().unwrap()[0]["following"];
    let following_after_arr = following_after.as_array().unwrap();
    assert_eq!(
        following_after_arr.len(),
        1,
        "Alice should now follow only 1 person"
    );

    // 11. Delete all likes from Carol
    run(&storage, "DELETE user:carol->likes");

    // Verify deletion - p1 should now only have 1 liker (Bob)
    let p1_likers_after = run(&storage, "SELECT <-likes<-user.name AS likers FROM post:p1");
    let likers_after = &p1_likers_after.as_array().unwrap()[0]["likers"];
    let likers_after_arr = likers_after.as_array().unwrap();
    assert_eq!(likers_after_arr.len(), 1, "Post p1 should now have 1 liker");
}

// =============================================================================
// Graph depth traversal test
// =============================================================================

#[test]
fn test_graph_depth_traversal() {
    let (_tmp, storage) = setup();

    // Create a chain: A -> B -> C -> D
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    run(&storage, "RELATE user:a->knows->user:b");
    run(&storage, "RELATE user:b->knows->user:c");
    run(&storage, "RELATE user:c->knows->user:d");

    // Single hop from A should give B
    let hop1 = run(&storage, "SELECT ->knows->user.* AS result FROM user:a");
    let r1 = &hop1.as_array().unwrap()[0]["result"];
    let arr1 = r1.as_array().unwrap();
    assert_eq!(arr1.len(), 1, "Single hop should give 1 result");
    assert_eq!(arr1[0]["name"], "B");

    // Two hops using chained traversal: A -> B -> C
    // Note: ->knows->knows->user.* means: follow knows twice, then get target user
    let hop2 = run(
        &storage,
        "SELECT ->knows->knows->user.* AS result FROM user:a",
    );
    let r2 = &hop2.as_array().unwrap()[0]["result"];
    let arr2 = r2.as_array().unwrap();
    assert_eq!(arr2.len(), 1, "Two hops should give 1 result (C)");
    assert_eq!(arr2[0]["name"], "C");
}

// =============================================================================
// Edge data test
// =============================================================================

#[test]
fn test_graph_with_edge_data() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // RELATE returns all edge fields including custom data
    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024, notes = 'Met at conference'",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["since"], 2024);
    assert_eq!(edges[0]["notes"], "Met at conference");

    // Note: Grammar limitation - ->follows.* syntax for edge fields is not supported.
    // Use RELATE output or ->follows->user.* for node fields instead.
    // Verify edge exists via traversal (returns edge ID only)
    let check = run(&storage, "SELECT ->follows AS edges FROM user:alice");
    let edge_arr = &check.as_array().unwrap()[0]["edges"].as_array().unwrap();
    assert_eq!(edge_arr.len(), 1);
    // Edge ID format is label:nanoid
    let edge_id = edge_arr[0]["id"].as_str().unwrap();
    assert!(
        edge_id.starts_with("follows:"),
        "Edge ID should start with 'follows:', got: {edge_id}"
    );
}

// =============================================================================
// Self-referential edge test
// =============================================================================

#[test]
fn test_graph_self_referential() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:narcissist SET name = 'Me'");

    // RELATE returns all edge fields including custom data
    let result = run(
        &storage,
        "RELATE user:narcissist->follows->user:narcissist SET reason = 'Self-love'",
    );
    let edges = result.as_array().unwrap();
    // Edge is created and points to same node
    assert_eq!(edges.len(), 1, "Self-referential edge should be created");
    assert_eq!(edges[0]["reason"], "Self-love");
    assert_eq!(edges[0]["from"], "user:narcissist");
    assert_eq!(edges[0]["to"], "user:narcissist");

    // Self-referential edges are now correctly returned in traversal
    // because edge deduplication is based on edge.id, not target node
    let check = run(&storage, "SELECT ->follows AS edges FROM user:narcissist");
    let edge_arr = &check.as_array().unwrap()[0]["edges"].as_array().unwrap();
    assert_eq!(
        edge_arr.len(),
        1,
        "Self-referential edges should be returned"
    );
    assert_eq!(edge_arr[0]["reason"], "Self-love");
}

// =============================================================================
// Cross-collection traversal test
// =============================================================================

#[test]
fn test_graph_cross_collection() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");
    run(&storage, "DEFINE COLLECTION comment");

    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(
        &storage,
        "CREATE post:p1 SET title = 'Hello', content = 'World'",
    );
    run(
        &storage,
        "CREATE comment:c1 SET text = 'Great post!', author = 'Bob'",
    );

    // Alice writes post
    run(&storage, "RELATE user:alice->wrote->post:p1");
    // Post has comment
    run(&storage, "RELATE comment:c1->on->post:p1");

    // Get Alice's posts
    let alice_posts = run(&storage, "SELECT ->wrote->post.* AS posts FROM user:alice");
    let posts = &alice_posts.as_array().unwrap()[0]["posts"];
    let posts_arr = posts.as_array().unwrap();
    assert_eq!(posts_arr.len(), 1);
    assert_eq!(posts_arr[0]["title"], "Hello");

    // Get comments on post (incoming edges)
    let comments = run(&storage, "SELECT <-on<-comment.* AS comments FROM post:p1");
    let comments_arr = &comments.as_array().unwrap()[0]["comments"];
    let arr = comments_arr.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["text"], "Great post!");
}

// =============================================================================
// Multiple edge labels test
// =============================================================================

#[test]
fn test_graph_multiple_labels() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edges with different labels
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->knows->user:bob");
    run(&storage, "RELATE user:alice->likes->user:bob");

    // Query each label separately
    let follows = run(&storage, "SELECT ->follows AS f FROM user:alice");
    let knows = run(&storage, "SELECT ->knows AS k FROM user:alice");
    let likes = run(&storage, "SELECT ->likes AS l FROM user:alice");

    assert_eq!(
        follows.as_array().unwrap()[0]["f"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        knows.as_array().unwrap()[0]["k"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        likes.as_array().unwrap()[0]["l"].as_array().unwrap().len(),
        1
    );
}

// =============================================================================
// Bidirectional traversal test
// =============================================================================

#[test]
fn test_graph_bidirectional() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // Alice follows Bob
    run(&storage, "RELATE user:alice->follows->user:bob");
    // Carol follows Alice
    run(&storage, "RELATE user:carol->follows->user:alice");

    // Outgoing from Alice (who Alice follows)
    let out = run(
        &storage,
        "SELECT ->follows->user.* AS following FROM user:alice",
    );
    let following = out.as_array().unwrap()[0]["following"].as_array().unwrap();
    assert_eq!(following.len(), 1);
    assert_eq!(following[0]["name"], "Bob");

    // Incoming to Alice (who follows Alice)
    let in_edges = run(
        &storage,
        "SELECT <-follows<-user.* AS followers FROM user:alice",
    );
    let followers = in_edges.as_array().unwrap()[0]["followers"]
        .as_array()
        .unwrap();
    assert_eq!(followers.len(), 1);
    assert_eq!(followers[0]["name"], "Carol");

    // Bidirectional (both directions)
    let both = run(
        &storage,
        "SELECT <->follows<->user.* AS connections FROM user:alice",
    );
    let connections = both.as_array().unwrap()[0]["connections"]
        .as_array()
        .unwrap();
    assert_eq!(
        connections.len(),
        2,
        "Should have 2 connections (Bob outgoing, Carol incoming)"
    );
}

// =============================================================================
// Edge upsert test
// =============================================================================

#[test]
fn test_graph_edge_upsert() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    // Create edge - RELATE returns all edge fields
    let result1 = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2023",
    );
    let edges1 = result1.as_array().unwrap();
    assert_eq!(edges1.len(), 1);
    assert_eq!(edges1[0]["since"], 2023);

    // Upsert (create same edge with different data) - RELATE returns updated edge
    let result2 = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024",
    );
    let edges2 = result2.as_array().unwrap();
    assert_eq!(edges2.len(), 1);
    assert_eq!(edges2[0]["since"], 2024, "Edge should be updated");

    // Verify only one edge exists via traversal
    let check = run(&storage, "SELECT ->follows AS edges FROM user:alice");
    let edge_arr = check.as_array().unwrap()[0]["edges"].as_array().unwrap();
    assert_eq!(edge_arr.len(), 1, "Should still have only 1 edge");
}

// =============================================================================
// Delete all edges by label test
// =============================================================================

#[test]
fn test_graph_delete_all_by_label() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");
    run(&storage, "CREATE user:dave SET name = 'Dave'");

    // Alice follows everyone
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");
    run(&storage, "RELATE user:alice->follows->user:dave");

    // Verify 3 edges
    let edges = run(&storage, "SELECT ->follows AS f FROM user:alice");
    assert_eq!(
        edges.as_array().unwrap()[0]["f"].as_array().unwrap().len(),
        3
    );

    // Delete all follows from Alice
    let result = run(&storage, "DELETE user:alice->follows");
    // DELETE edge returns [{"deleted": count}]
    let arr = result.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["deleted"], 3);

    // Verify all edges gone
    let edges_after = run(&storage, "SELECT ->follows AS f FROM user:alice");
    assert_eq!(
        edges_after.as_array().unwrap()[0]["f"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

// =============================================================================
// Cross-collection incoming edges test
// =============================================================================

#[test]
fn test_graph_cross_collection_incoming() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE post:p1 SET title = 'Hello'");

    // Create likes edges: user -> post
    run(&storage, "RELATE user:alice->likes->post:p1");
    run(&storage, "RELATE user:bob->likes->post:p1");

    // Query outgoing from alice
    let out = run(&storage, "SELECT ->likes AS edges FROM user:alice");
    let out_arr = &out.as_array().unwrap()[0]["edges"].as_array().unwrap();
    assert_eq!(out_arr.len(), 1, "Alice has 1 likes edge");

    // Query incoming to post:p1 - just edge data
    let inn = run(&storage, "SELECT <-likes AS edges FROM post:p1");
    let inn_arr = &inn.as_array().unwrap()[0]["edges"].as_array().unwrap();
    assert_eq!(inn_arr.len(), 2, "Post p1 has 2 incoming likes edges");

    // Query incoming with user.* target (needs .* to be recognized as node target)
    // Note: Without .* or .field, "user" would be parsed as an edge label, not a node target
    let likers = run(&storage, "SELECT <-likes<-user.* AS likers FROM post:p1");
    let likers_arr = &likers.as_array().unwrap()[0]["likers"].as_array().unwrap();
    assert_eq!(likers_arr.len(), 2, "Post p1 has 2 likers");
}

// =============================================================================
// Complex social network scenario
// =============================================================================

#[test]
fn test_graph_social_network_scenario() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    // Create users
    run(
        &storage,
        "CREATE user:alice SET name = 'Alice', role = 'admin'",
    );
    run(&storage, "CREATE user:bob SET name = 'Bob', role = 'user'");
    run(
        &storage,
        "CREATE user:carol SET name = 'Carol', role = 'moderator'",
    );
    run(
        &storage,
        "CREATE user:dave SET name = 'Dave', role = 'user'",
    );

    // Create posts
    run(
        &storage,
        "CREATE post:p1 SET title = 'Welcome to StellarDB', views = 100",
    );
    run(
        &storage,
        "CREATE post:p2 SET title = 'Graph Databases 101', views = 50",
    );
    run(
        &storage,
        "CREATE post:p3 SET title = 'Tips for Rust', views = 75",
    );

    // Social graph
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");
    run(&storage, "RELATE user:bob->follows->user:carol");
    run(&storage, "RELATE user:carol->follows->user:alice");
    run(&storage, "RELATE user:dave->follows->user:alice");
    run(&storage, "RELATE user:dave->follows->user:bob");

    // Authorship
    run(&storage, "RELATE user:alice->wrote->post:p1");
    run(&storage, "RELATE user:bob->wrote->post:p2");
    run(&storage, "RELATE user:carol->wrote->post:p3");

    // Likes
    run(&storage, "RELATE user:alice->likes->post:p2");
    run(&storage, "RELATE user:alice->likes->post:p3");
    run(&storage, "RELATE user:bob->likes->post:p1");
    run(&storage, "RELATE user:bob->likes->post:p3");
    run(&storage, "RELATE user:carol->likes->post:p1");
    run(&storage, "RELATE user:carol->likes->post:p2");
    run(&storage, "RELATE user:dave->likes->post:p1");
    run(&storage, "RELATE user:dave->likes->post:p2");
    run(&storage, "RELATE user:dave->likes->post:p3");

    // Find all posts liked by people Alice follows
    let alice_following = run(&storage, "SELECT ->follows->user.* AS f FROM user:alice");
    let following_arr = alice_following.as_array().unwrap()[0]["f"]
        .as_array()
        .unwrap();
    assert_eq!(following_arr.len(), 2, "Alice follows 2 people");

    // Find who follows Alice
    let alice_followers = run(&storage, "SELECT <-follows<-user.* AS f FROM user:alice");
    let followers_arr = alice_followers.as_array().unwrap()[0]["f"]
        .as_array()
        .unwrap();
    assert_eq!(
        followers_arr.len(),
        2,
        "2 people follow Alice (Carol and Dave)"
    );

    // Count likes on each post (use <-likes for edge count since <-likes<-user
    // would look for edges with label "user" instead of user nodes)
    let p1_likes = run(&storage, "SELECT <-likes AS likers FROM post:p1");
    let p1_likers = p1_likes.as_array().unwrap()[0]["likers"]
        .as_array()
        .unwrap();
    assert_eq!(p1_likers.len(), 3, "Post p1 has 3 likes");

    let p2_likes = run(&storage, "SELECT <-likes AS likers FROM post:p2");
    let p2_likers = p2_likes.as_array().unwrap()[0]["likers"]
        .as_array()
        .unwrap();
    assert_eq!(p2_likers.len(), 3, "Post p2 has 3 likes");

    let p3_likes = run(&storage, "SELECT <-likes AS likers FROM post:p3");
    let p3_likers = p3_likes.as_array().unwrap()[0]["likers"]
        .as_array()
        .unwrap();
    assert_eq!(p3_likers.len(), 3, "Post p3 has 3 likes");
}

// =============================================================================
// Array RELATE test
// =============================================================================

#[test]
fn test_graph_array_relate() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    // Alice follows multiple users at once
    let result = run(
        &storage,
        "RELATE user:alice->follows->[user:bob, user:carol]",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 2, "Should create 2 edges");

    // Verify both edges exist
    let following = run(&storage, "SELECT ->follows AS f FROM user:alice");
    let f_arr = following.as_array().unwrap()[0]["f"].as_array().unwrap();
    assert_eq!(f_arr.len(), 2);
}

// =============================================================================
// Cartesian product RELATE test
// =============================================================================

#[test]
fn test_graph_cartesian_relate() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");
    run(&storage, "CREATE user:dave SET name = 'Dave'");

    // 2 sources x 2 targets = 4 edges
    let result = run(
        &storage,
        "RELATE [user:alice, user:bob]->follows->[user:carol, user:dave]",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 4, "Should create 4 edges (cartesian product)");

    // Verify Alice's outgoing edges
    let alice_following = run(&storage, "SELECT ->follows AS f FROM user:alice");
    let alice_f = alice_following.as_array().unwrap()[0]["f"]
        .as_array()
        .unwrap();
    assert_eq!(alice_f.len(), 2, "Alice should follow 2 people");

    // Verify Bob's outgoing edges
    let bob_following = run(&storage, "SELECT ->follows AS f FROM user:bob");
    let bob_f = bob_following.as_array().unwrap()[0]["f"]
        .as_array()
        .unwrap();
    assert_eq!(bob_f.len(), 2, "Bob should follow 2 people");

    // Verify Carol's incoming edges
    let carol_followers = run(&storage, "SELECT <-follows AS f FROM user:carol");
    let carol_f = carol_followers.as_array().unwrap()[0]["f"]
        .as_array()
        .unwrap();
    assert_eq!(carol_f.len(), 2, "Carol should have 2 followers");
}

// =============================================================================
// RETURN clause variations
// =============================================================================

#[test]
fn test_graph_relate_return_none() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(&storage, "RELATE user:alice->follows->user:bob RETURN NONE");
    // Result is now wrapped in array: [{"count": 1}]
    let arr = result.as_array().expect("Should return array");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["count"], 1);
}

#[test]
fn test_graph_relate_return_after() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET score = 10 RETURN AFTER",
    );
    let edges = result.as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["score"], 10);
}

// =============================================================================
// Edge fields test
// =============================================================================

#[test]
fn test_graph_edge_default_fields() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(
        &storage,
        "RELATE user:alice->follows->user:bob SET weight = 5",
    );
    let edges = result.as_array().unwrap();

    // Edge should have id, from, to fields by default
    assert!(edges[0]["id"].is_string(), "Edge should have id");
    assert_eq!(edges[0]["from"], "user:alice");
    assert_eq!(edges[0]["to"], "user:bob");
    assert_eq!(edges[0]["weight"], 5);
}

// =============================================================================
// Edge ID format test
// =============================================================================

#[test]
fn test_graph_edge_id_format() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    let result = run(&storage, "RELATE user:alice->follows->user:bob");
    let edges = result.as_array().unwrap();
    let id = edges[0]["id"].as_str().unwrap();
    // Edge ID format is label:nanoid (e.g., "follows:TAdhA4qzHUPCSMMfFter6")
    assert!(
        id.starts_with("follows:"),
        "Edge ID should start with label prefix, got: {id}"
    );
    // Nanoid is 21 characters
    let nanoid_part = &id["follows:".len()..];
    assert_eq!(
        nanoid_part.len(),
        21,
        "Edge ID nanoid should be 21 chars, got: {nanoid_part}"
    );
}

// =============================================================================
// Aggregate function error messages for expression arguments
// =============================================================================

#[test]
fn test_count_with_traversal_error_message() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "RELATE user:alice->follows->user:bob");

    let err = run_err(&storage, "SELECT COUNT(->follows) AS c FROM user:alice");
    assert!(
        err.contains("COUNT with expression argument is not an aggregate"),
        "Expected helpful error message, got: {err}"
    );
    assert!(
        err.contains("array::length"),
        "Expected suggestion to use array::length, got: {err}"
    );
}

#[test]
fn test_sum_with_expression_error_message() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION test");
    run(&storage, "CREATE test:1 SET nums = [1, 2, 3]");

    // SUM([1,2,3]) - array literal, not a field reference
    let err = run_err(&storage, "SELECT SUM([1,2,3]) AS s FROM test:1");
    assert!(
        err.contains("SUM with expression argument is not supported"),
        "Expected helpful error message, got: {err}"
    );
    assert!(
        err.contains("Aggregates like SUM(field) only work on field references"),
        "Expected explanation about aggregates, got: {err}"
    );
}

#[test]
fn test_avg_with_expression_error_message() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION test");
    run(&storage, "CREATE test:1 SET nums = [1, 2, 3]");

    let err = run_err(&storage, "SELECT AVG([1,2,3]) AS a FROM test:1");
    assert!(
        err.contains("AVG with expression argument is not supported"),
        "Expected helpful error message, got: {err}"
    );
}

#[test]
fn test_min_with_expression_error_message() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION test");
    run(&storage, "CREATE test:1 SET nums = [1, 2, 3]");

    let err = run_err(&storage, "SELECT MIN([1,2,3]) AS m FROM test:1");
    assert!(
        err.contains("MIN with expression argument is not supported"),
        "Expected helpful error message, got: {err}"
    );
}

#[test]
fn test_max_with_expression_error_message() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION test");
    run(&storage, "CREATE test:1 SET nums = [1, 2, 3]");

    let err = run_err(&storage, "SELECT MAX([1,2,3]) AS m FROM test:1");
    assert!(
        err.contains("MAX with expression argument is not supported"),
        "Expected helpful error message, got: {err}"
    );
}

#[test]
fn test_array_length_works_for_traversal() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    let result = run(
        &storage,
        "SELECT array::length(->follows) AS follow_count FROM user:alice",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);
    assert_eq!(data[0]["follow_count"], 2);
}

// =============================================================================
// Traversal returns all edge fields by default
// =============================================================================

#[test]
fn test_traversal_returns_edge_fields_by_default() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024",
    );

    // Traversal should return all edge fields: id, from, to + user fields
    let result = run(&storage, "SELECT ->follows AS edges FROM user:alice");
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let edges = data[0]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);

    let edge = &edges[0];
    assert!(edge["id"].is_string(), "Edge should have id");
    assert_eq!(edge["from"], "user:alice", "Edge should have from");
    assert_eq!(edge["to"], "user:bob", "Edge should have to");
    assert_eq!(edge["since"], 2024, "Edge should have user fields");
}

// =============================================================================
// Multi-field traversal selection tests
// =============================================================================

#[test]
fn test_traversal_edge_multi_field_selection() {
    let (_tmp, storage) = setup();

    // Setup: Create recipe and ingredient with edge containing quantity data
    run(&storage, "DEFINE COLLECTION recipe");
    run(&storage, "DEFINE COLLECTION ingredient");
    run(&storage, "CREATE recipe:cake SET name = 'Chocolate Cake'");
    run(
        &storage,
        "CREATE ingredient:flour SET name = 'All-Purpose Flour'",
    );
    run(
        &storage,
        "CREATE ingredient:sugar SET name = 'Granulated Sugar'",
    );
    run(
        &storage,
        "RELATE recipe:cake->contains->ingredient:flour SET quantity_grams = 250, unit = 'grams'",
    );
    run(
        &storage,
        "RELATE recipe:cake->contains->ingredient:sugar SET quantity_grams = 200, unit = 'grams'",
    );

    // Query with multi-field selection (aliases)
    let result = run(
        &storage,
        "SELECT ->contains.{grams: quantity_grams, target: to} AS ingredients FROM recipe:cake",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let ingredients = data[0]["ingredients"].as_array().unwrap();
    assert_eq!(ingredients.len(), 2);

    // Check that both ingredients have the aliased fields
    for ingredient in ingredients {
        assert!(
            ingredient["grams"].is_number(),
            "Should have grams alias for quantity_grams: {:?}",
            ingredient
        );
        assert!(
            ingredient["target"].is_string(),
            "Should have target alias for to: {:?}",
            ingredient
        );
    }

    // Check specific values
    let grams_values: Vec<i64> = ingredients
        .iter()
        .map(|i| i["grams"].as_i64().unwrap())
        .collect();
    assert!(grams_values.contains(&250), "Should have 250g flour");
    assert!(grams_values.contains(&200), "Should have 200g sugar");
}

#[test]
fn test_traversal_edge_fields_without_alias() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024, notes = 'Met at conference'",
    );

    // Query with multi-field selection (no aliases)
    let result = run(
        &storage,
        "SELECT ->follows.{since, notes, to} AS edges FROM user:alice",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let edges = data[0]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);

    let edge = &edges[0];
    assert_eq!(edge["since"], 2024);
    assert_eq!(edge["notes"], "Met at conference");
    assert_eq!(edge["to"], "user:bob");
}

#[test]
fn test_traversal_edge_to_field_as_reference() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(
        &storage,
        "CREATE user:bob SET name = 'Bob', email = 'bob@example.com'",
    );
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2024",
    );

    // Query that selects 'to' field - should return as reference string
    let result = run(&storage, "SELECT ->follows.{to} AS edges FROM user:alice");
    let data = result.as_array().unwrap();
    let edges = data[0]["edges"].as_array().unwrap();
    assert_eq!(edges.len(), 1);

    // 'to' should be a reference string (e.g., "user:bob")
    let to_ref = edges[0]["to"].as_str().unwrap();
    assert_eq!(to_ref, "user:bob", "to field should be a reference string");
}

#[test]
fn test_traversal_node_multi_field_selection() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice', age = 30");
    run(
        &storage,
        "CREATE user:bob SET name = 'Bob', age = 25, email = 'bob@example.com'",
    );
    run(
        &storage,
        "CREATE user:carol SET name = 'Carol', age = 28, email = 'carol@example.com'",
    );
    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    // Query with multi-field selection on target nodes
    let result = run(
        &storage,
        "SELECT ->follows->user.{name, email} AS following FROM user:alice",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let following = data[0]["following"].as_array().unwrap();
    assert_eq!(following.len(), 2);

    // Check that only name and email are returned, not age
    for user in following {
        assert!(user["name"].is_string(), "Should have name field");
        assert!(user["email"].is_string(), "Should have email field");
        // age should not be present when selecting specific fields
        assert!(
            user.get("age").is_none() || user["age"].is_null(),
            "Should not have age field when selecting specific fields"
        );
    }
}

#[test]
fn test_traversal_node_fields_with_alias() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(
        &storage,
        "CREATE user:bob SET name = 'Bob', email = 'bob@example.com'",
    );
    run(&storage, "RELATE user:alice->follows->user:bob");

    // Query with aliased fields on target nodes
    let result = run(
        &storage,
        "SELECT ->follows->user.{full_name: name, contact: email} AS friend FROM user:alice",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let friends = data[0]["friend"].as_array().unwrap();
    assert_eq!(friends.len(), 1);

    let friend = &friends[0];
    assert_eq!(
        friend["full_name"], "Bob",
        "Should have aliased full_name field"
    );
    assert_eq!(
        friend["contact"], "bob@example.com",
        "Should have aliased contact field"
    );
    // Original field names should not be present
    assert!(
        friend.get("name").is_none() || friend["name"].is_null(),
        "Original 'name' should not be present when aliased"
    );
}

// =============================================================================
// Multi-step traversal with intermediate collection filter
// =============================================================================

#[test]
fn test_traversal_three_steps_with_collection_filter() {
    let (_tmp, storage) = setup();

    // Setup: E-commerce scenario
    // customer -> viewed -> product
    // product -> bought_together -> product
    run(&storage, "DEFINE COLLECTION customer");
    run(&storage, "DEFINE COLLECTION product");

    run(&storage, "CREATE customer:1 SET name = 'John'");
    run(
        &storage,
        "CREATE product:laptop SET name = 'Laptop', price = 999",
    );
    run(
        &storage,
        "CREATE product:mouse SET name = 'Mouse', price = 29",
    );
    run(
        &storage,
        "CREATE product:keyboard SET name = 'Keyboard', price = 79",
    );
    run(
        &storage,
        "CREATE product:monitor SET name = 'Monitor', price = 299",
    );

    // Customer viewed products
    run(
        &storage,
        "RELATE customer:1->viewed->product:laptop SET at = 1",
    );
    run(
        &storage,
        "RELATE customer:1->viewed->product:mouse SET at = 2",
    );

    // Products bought together
    run(
        &storage,
        "RELATE product:laptop->bought_together->product:mouse SET count = 100, confidence = 0.85",
    );
    run(
        &storage,
        "RELATE product:laptop->bought_together->product:keyboard SET count = 75, confidence = 0.65",
    );
    run(
        &storage,
        "RELATE product:mouse->bought_together->product:keyboard SET count = 50, confidence = 0.45",
    );

    // Test: Three-step traversal with intermediate collection filter
    // customer:1 -> viewed -> product -> bought_together
    // "product" is a collection name, so it acts as a filter for intermediate nodes
    let result = run(
        &storage,
        "SELECT ->viewed->product->bought_together.* AS cross_sell FROM customer:1",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let cross_sell = data[0]["cross_sell"].as_array().unwrap();
    // Should get edges from products that customer viewed:
    // - laptop -> bought_together -> mouse (count=100)
    // - laptop -> bought_together -> keyboard (count=75)
    // - mouse -> bought_together -> keyboard (count=50)
    assert_eq!(
        cross_sell.len(),
        3,
        "Should get 3 bought_together edges from viewed products"
    );

    // Verify the edges have expected fields
    let counts: Vec<i64> = cross_sell
        .iter()
        .map(|e| e["count"].as_i64().unwrap())
        .collect();
    assert!(
        counts.contains(&100),
        "Should have laptop->mouse edge (count=100)"
    );
    assert!(
        counts.contains(&75),
        "Should have laptop->keyboard edge (count=75)"
    );
    assert!(
        counts.contains(&50),
        "Should have mouse->keyboard edge (count=50)"
    );
}

#[test]
fn test_traversal_three_steps_with_field_selection() {
    let (_tmp, storage) = setup();

    // Setup
    run(&storage, "DEFINE COLLECTION customer");
    run(&storage, "DEFINE COLLECTION product");

    run(&storage, "CREATE customer:1 SET name = 'John'");
    run(&storage, "CREATE product:laptop SET name = 'Laptop'");
    run(&storage, "CREATE product:mouse SET name = 'Mouse'");

    run(&storage, "RELATE customer:1->viewed->product:laptop");
    run(
        &storage,
        "RELATE product:laptop->bought_together->product:mouse SET count = 100, confidence = 0.85",
    );

    // Test: Three-step traversal with multi-field selection on edges
    let result = run(
        &storage,
        "SELECT ->viewed->product->bought_together.{times: count, confidence, product_name: to.name} AS suggestions FROM customer:1",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let suggestions = data[0]["suggestions"].as_array().unwrap();
    assert_eq!(suggestions.len(), 1);

    let suggestion = &suggestions[0];
    assert_eq!(suggestion["times"], 100, "count aliased as times");
    assert_eq!(suggestion["confidence"], 0.85);
    assert_eq!(
        suggestion["product_name"], "Mouse",
        "to.name dereferenced and aliased"
    );
}

#[test]
fn test_traversal_four_steps_chain() {
    let (_tmp, storage) = setup();

    // Setup: user -> follows -> user -> wrote -> post
    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION post");

    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");
    run(&storage, "CREATE post:p1 SET title = 'Hello World'");
    run(&storage, "CREATE post:p2 SET title = 'Rust Tips'");

    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");
    run(&storage, "RELATE user:bob->wrote->post:p1");
    run(&storage, "RELATE user:carol->wrote->post:p2");

    // Three-step traversal: alice -> follows -> user -> wrote -> post
    let result = run(
        &storage,
        "SELECT ->follows->user->wrote->post.* AS friend_posts FROM user:alice",
    );
    let data = result.as_array().unwrap();
    assert_eq!(data.len(), 1);

    let friend_posts = data[0]["friend_posts"].as_array().unwrap();
    assert_eq!(
        friend_posts.len(),
        2,
        "Should get posts from both Bob and Carol"
    );

    let titles: Vec<&str> = friend_posts
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Hello World"), "Should have Bob's post");
    assert!(titles.contains(&"Rust Tips"), "Should have Carol's post");
}

// =============================================================================
// SELECT VALUE + graph traversal tests
// =============================================================================

#[test]
fn test_select_value_traversal() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob', age = 25");
    run(&storage, "CREATE user:carol SET name = 'Carol', age = 28");

    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    // SELECT VALUE with traversal should return flat array of user objects
    let result = run(
        &storage,
        "SELECT VALUE ->follows->user.* AS similar FROM user:alice",
    );
    let arr = result.as_array().unwrap();

    // Should be a flat array of user objects (not wrapped in { "similar": [...] })
    assert_eq!(arr.len(), 2, "Should return 2 followed users");

    // Each element should be a user object with fields
    let names: Vec<&str> = arr.iter().map(|u| u["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Bob"), "Should contain Bob");
    assert!(names.contains(&"Carol"), "Should contain Carol");
}

#[test]
fn test_select_value_traversal_single_field() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    // SELECT VALUE with traversal selecting a single field should return flat array of values
    // ->follows->user.name returns user objects with only the `name` field selected
    let result = run(
        &storage,
        "SELECT VALUE ->follows->user.name FROM user:alice",
    );
    let arr = result.as_array().unwrap();

    // Should be a flat array of user objects with name field
    assert_eq!(arr.len(), 2, "Should return 2 users");

    let names: Vec<&str> = arr.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"Bob"), "Should contain Bob");
    assert!(names.contains(&"Carol"), "Should contain Carol");
}

#[test]
fn test_select_value_traversal_with_function() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:carol SET name = 'Carol'");

    run(&storage, "RELATE user:alice->follows->user:bob");
    run(&storage, "RELATE user:alice->follows->user:carol");

    // SELECT VALUE with array::length wrapping a traversal
    let result = run(
        &storage,
        "SELECT VALUE array::length(->follows->user.*) FROM user:alice",
    );
    let arr = result.as_array().unwrap();

    // Should return [2] - the count of followed users
    assert_eq!(arr.len(), 1, "Should return single value");
    assert_eq!(arr[0], 2, "Should be 2 (count of followed users)");
}

// ============================================================
// Edge filter tests
// ============================================================

#[test]
fn test_edge_filter_basic() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:charlie SET name = 'Charlie'");

    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2020, mutual = true",
    );
    run(
        &storage,
        "RELATE user:alice->follows->user:charlie SET since = 2024, mutual = false",
    );

    // Filter edges by since > 2022 — should only get charlie
    let result = run(
        &storage,
        "SELECT ->(follows WHERE since > 2022)->user.* AS filtered FROM user:alice",
    );
    let filtered = result.as_array().unwrap()[0]["filtered"]
        .as_array()
        .unwrap();
    assert_eq!(filtered.len(), 1, "Should match only one edge");
    assert_eq!(filtered[0]["name"], "Charlie");
}

#[test]
fn test_edge_filter_boolean() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:charlie SET name = 'Charlie'");

    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2020, mutual = true",
    );
    run(
        &storage,
        "RELATE user:alice->follows->user:charlie SET since = 2024, mutual = false",
    );

    // Filter by mutual = true — should only get bob
    let result = run(
        &storage,
        "SELECT ->(follows WHERE mutual = true)->user.* AS mutual_friends FROM user:alice",
    );
    let friends = result.as_array().unwrap()[0]["mutual_friends"]
        .as_array()
        .unwrap();
    assert_eq!(friends.len(), 1, "Should match only mutual follows");
    assert_eq!(friends[0]["name"], "Bob");
}

#[test]
fn test_edge_filter_no_match() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");

    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET since = 2020",
    );

    // Filter with impossible condition — should get empty array
    let result = run(
        &storage,
        "SELECT ->(follows WHERE since > 3000)->user.* AS none FROM user:alice",
    );
    let none = result.as_array().unwrap()[0]["none"].as_array().unwrap();
    assert_eq!(none.len(), 0, "No edges should match");
}

#[test]
fn test_edge_filter_multi_hop() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "DEFINE COLLECTION product");
    run(&storage, "CREATE user:alice SET name = 'Alice'");
    run(&storage, "CREATE user:bob SET name = 'Bob'");
    run(&storage, "CREATE user:charlie SET name = 'Charlie'");
    run(&storage, "CREATE product:p1 SET name = 'Widget'");
    run(&storage, "CREATE product:p2 SET name = 'Gadget'");

    // Alice follows Bob (mutual) and Charlie (not mutual)
    run(
        &storage,
        "RELATE user:alice->follows->user:bob SET mutual = true",
    );
    run(
        &storage,
        "RELATE user:alice->follows->user:charlie SET mutual = false",
    );

    // Both Bob and Charlie like products
    run(&storage, "RELATE user:bob->likes->product:p1");
    run(&storage, "RELATE user:charlie->likes->product:p2");

    // Filter to only mutual follows, then traverse likes
    // Should only get product:p1 (via Bob)
    let result = run(
        &storage,
        "SELECT ->(follows WHERE mutual = true)->user->likes->product.* AS products FROM user:alice",
    );
    let products = result.as_array().unwrap()[0]["products"]
        .as_array()
        .unwrap();
    assert_eq!(
        products.len(),
        1,
        "Should only get products via mutual follows"
    );
    assert_eq!(products[0]["name"], "Widget");
}

// ============================================================
// Min depth tests
// ============================================================

#[test]
fn test_depth_exact() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    // Chain: a -> b -> c -> d
    run(&storage, "RELATE user:a->knows->user:b");
    run(&storage, "RELATE user:b->knows->user:c");
    run(&storage, "RELATE user:c->knows->user:d");

    // Exact depth 2: a -> b -> c (skip a->b at depth 1)
    let result = run(&storage, "SELECT ->knows{2}->user.* AS found FROM user:a");
    let found = result.as_array().unwrap()[0]["found"].as_array().unwrap();
    assert_eq!(found.len(), 1, "Exact depth 2 should return 1 node");
    assert_eq!(found[0]["name"], "C");
}

#[test]
fn test_depth_range_min() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    // Chain: a -> b -> c -> d
    run(&storage, "RELATE user:a->knows->user:b");
    run(&storage, "RELATE user:b->knows->user:c");
    run(&storage, "RELATE user:c->knows->user:d");

    // Range {2..3}: should get C (depth 2) and D (depth 3), but NOT B (depth 1)
    let result = run(
        &storage,
        "SELECT ->knows{2..3}->user.* AS found FROM user:a",
    );
    let found = result.as_array().unwrap()[0]["found"].as_array().unwrap();
    assert_eq!(found.len(), 2, "Range {{2..3}} should return 2 nodes");

    let names: Vec<&str> = found.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"C"), "Should contain C (depth 2)");
    assert!(names.contains(&"D"), "Should contain D (depth 3)");
}

#[test]
fn test_depth_range_max_only() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    // Chain: a -> b -> c -> d
    run(&storage, "RELATE user:a->knows->user:b");
    run(&storage, "RELATE user:b->knows->user:c");
    run(&storage, "RELATE user:c->knows->user:d");

    // {..2}: should get B (depth 1) and C (depth 2), but NOT D (depth 3)
    let result = run(&storage, "SELECT ->knows{..2}->user.* AS found FROM user:a");
    let found = result.as_array().unwrap()[0]["found"].as_array().unwrap();
    assert_eq!(found.len(), 2, "Range {{..2}} should return 2 nodes");

    let names: Vec<&str> = found.iter().map(|v| v["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"B"), "Should contain B (depth 1)");
    assert!(names.contains(&"C"), "Should contain C (depth 2)");
}

#[test]
fn test_depth_min_only() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");

    // Chain: a -> b -> c -> d
    run(&storage, "RELATE user:a->knows->user:b");
    run(&storage, "RELATE user:b->knows->user:c");
    run(&storage, "RELATE user:c->knows->user:d");

    // {3..}: min depth 3, should get D only (max defaults to 10)
    let result = run(&storage, "SELECT ->knows{3..}->user.* AS found FROM user:a");
    let found = result.as_array().unwrap()[0]["found"].as_array().unwrap();
    assert_eq!(found.len(), 1, "Range {{3..}} should return 1 node");
    assert_eq!(found[0]["name"], "D");
}

#[test]
fn test_edge_filter_with_depth() {
    let (_tmp, storage) = setup();

    run(&storage, "DEFINE COLLECTION user");
    run(&storage, "CREATE user:a SET name = 'A'");
    run(&storage, "CREATE user:b SET name = 'B'");
    run(&storage, "CREATE user:c SET name = 'C'");
    run(&storage, "CREATE user:d SET name = 'D'");
    run(&storage, "CREATE user:e SET name = 'E'");

    // a->b (weight 10), b->c (weight 5), b->d (weight 20), c->e (weight 15)
    run(&storage, "RELATE user:a->knows->user:b SET weight = 10");
    run(&storage, "RELATE user:b->knows->user:c SET weight = 5");
    run(&storage, "RELATE user:b->knows->user:d SET weight = 20");
    run(&storage, "RELATE user:c->knows->user:e SET weight = 15");

    // Filter weight > 8, depth {1..3}: should get b (depth 1), d (depth 2), e (depth 3)
    // c is skipped because the edge b->c has weight=5 which doesn't pass filter
    let result = run(
        &storage,
        "SELECT ->(knows WHERE weight > 8){1..3}->user.* AS found FROM user:a",
    );
    let found = result.as_array().unwrap()[0]["found"].as_array().unwrap();

    let names: Vec<&str> = found.iter().map(|v| v["name"].as_str().unwrap()).collect();
    // a->b (weight 10 > 8, passes) = B at depth 1
    // b->c (weight 5, fails) = C filtered out
    // b->d (weight 20 > 8, passes) = D at depth 2
    // Since C is filtered, no path to E
    assert!(
        names.contains(&"B"),
        "Should contain B (depth 1, weight 10)"
    );
    assert!(
        names.contains(&"D"),
        "Should contain D (depth 2, weight 20)"
    );
    assert!(!names.contains(&"C"), "Should NOT contain C (weight 5 < 8)");
    assert!(
        !names.contains(&"E"),
        "Should NOT contain E (no path, C was filtered)"
    );
}
