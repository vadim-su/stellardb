//! Auth statement parsing: CREATE/DROP/ALTER USER, CREATE/DROP API KEY,
//! CREATE/DROP POLICY, ALTER COLLECTION SET/REMOVE attributes.

use pest::iterators::Pair;

use super::super::value::Rule;
use crate::query::ast::stmt::*;
use crate::query::error::ParseError;

/// Extract and decode a StellarQL string literal.
fn parse_string(pair: Pair<Rule>) -> Result<String, ParseError> {
    super::super::value::parse_string_literal(pair)
}

/// Parse CREATE USER 'name' PASSWORD 'pass'
pub(super) fn parse_create_user(pair: Pair<Rule>) -> Result<CreateUserAst, ParseError> {
    let mut strings = pair.into_inner().filter(|p| p.as_rule() == Rule::string);
    let username = parse_string(strings.next().expect("grammar"))?;
    let password = parse_string(strings.next().expect("grammar"))?;
    Ok(CreateUserAst { username, password })
}

/// Parse DROP USER 'name'
pub(super) fn parse_drop_user(pair: Pair<Rule>) -> Result<DropUserAst, ParseError> {
    let username = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::string)
        .map(parse_string)
        .transpose()?
        .expect("grammar");
    Ok(DropUserAst { username })
}

/// Parse ALTER USER 'name' (PASSWORD 'pass' | SET attrs | REMOVE attrs)
pub(super) fn parse_alter_user(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let mut username = String::new();
    let mut inner_iter = pair.into_inner();

    for p in inner_iter.by_ref() {
        match p.as_rule() {
            Rule::string => {
                username = parse_string(p)?;
            }
            Rule::alter_user_password => {
                let password = p
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::string)
                    .map(parse_string)
                    .transpose()?
                    .expect("grammar");
                return Ok(Statement::AlterUser(AlterUserAst::Password {
                    username,
                    password,
                }));
            }
            Rule::alter_user_set => {
                let attributes = parse_attr_pair_list(p)?;
                return Ok(Statement::AlterUser(AlterUserAst::Set {
                    username,
                    attributes,
                }));
            }
            Rule::alter_user_remove => {
                let attributes = parse_ident_list(p)?;
                return Ok(Statement::AlterUser(AlterUserAst::Remove {
                    username,
                    attributes,
                }));
            }
            _ => {}
        }
    }

    unreachable!("grammar guarantees one of the alter_user variants")
}

/// Parse CREATE API KEY 'name' FOR USER 'username'
pub(super) fn parse_create_api_key(pair: Pair<Rule>) -> Result<CreateApiKeyAst, ParseError> {
    let mut strings = pair.into_inner().filter(|p| p.as_rule() == Rule::string);
    let name = parse_string(strings.next().expect("grammar"))?;
    let username = parse_string(strings.next().expect("grammar"))?;
    Ok(CreateApiKeyAst { name, username })
}

/// Parse DROP API KEY 'name'
pub(super) fn parse_drop_api_key(pair: Pair<Rule>) -> Result<DropApiKeyAst, ParseError> {
    let name = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::string)
        .map(parse_string)
        .transpose()?
        .expect("grammar");
    Ok(DropApiKeyAst { name })
}

/// Parse CREATE POLICY name WHEN conditions... ALLOW|DENY
pub(super) fn parse_create_policy(pair: Pair<Rule>) -> Result<CreatePolicyAst, ParseError> {
    let mut name = String::new();
    let mut conditions = Vec::new();
    let mut effect = PolicyEffectAst::Deny;

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident => name = super::super::parse_ident(inner),
            Rule::policy_condition => {
                conditions.push(parse_policy_condition(inner)?);
            }
            Rule::policy_effect => {
                let eff_inner = inner.into_inner().next().expect("grammar");
                effect = match eff_inner.as_rule() {
                    Rule::kw_ALLOW => PolicyEffectAst::Allow,
                    Rule::kw_DENY => PolicyEffectAst::Deny,
                    _ => unreachable!("grammar"),
                };
            }
            _ => {}
        }
    }

    Ok(CreatePolicyAst {
        name,
        conditions,
        effect,
    })
}

/// Parse DROP POLICY name
pub(super) fn parse_drop_policy(pair: Pair<Rule>) -> Result<DropPolicyAst, ParseError> {
    let name = pair
        .into_inner()
        .find(|p| p.as_rule() == Rule::ident)
        .map(|p| super::super::parse_ident(p))
        .expect("grammar");
    Ok(DropPolicyAst { name })
}

/// Parse ALTER COLLECTION name (SET attrs | REMOVE attrs)
pub(super) fn parse_alter_collection_attrs(pair: Pair<Rule>) -> Result<Statement, ParseError> {
    let mut collection = String::new();

    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::ident => {
                collection = super::super::parse_ident(p);
            }
            Rule::alter_coll_set => {
                let attributes = parse_attr_pair_list(p)?;
                return Ok(Statement::AlterCollectionAttrs(
                    AlterCollectionAttrsAst::Set {
                        collection,
                        attributes,
                    },
                ));
            }
            Rule::alter_coll_remove => {
                let attributes = parse_ident_list(p)?;
                return Ok(Statement::AlterCollectionAttrs(
                    AlterCollectionAttrsAst::Remove {
                        collection,
                        attributes,
                    },
                ));
            }
            _ => {}
        }
    }

    unreachable!("grammar guarantees SET or REMOVE")
}

/// Parse a single policy_condition: field op value
fn parse_policy_condition(pair: Pair<Rule>) -> Result<PolicyConditionAst, ParseError> {
    let mut field = String::new();
    let mut op = String::new();
    let mut values = Vec::new();

    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::policy_field => {
                // policy_field = { ident ~ "." ~ ident | kw_ACTION }
                let children: Vec<_> = inner.into_inner().collect();
                if children.len() == 1 && children[0].as_rule() == Rule::kw_ACTION {
                    field = "action".to_string();
                } else {
                    // ident.ident
                    let parts: Vec<String> = children
                        .into_iter()
                        .filter(|p| p.as_rule() == Rule::ident)
                        .map(|p| super::super::parse_ident(p))
                        .collect();
                    field = parts.join(".");
                }
            }
            Rule::policy_op => {
                // policy_op = { "!=" | "=" | kw_IN }
                let raw = inner.as_str().trim();
                if raw.eq_ignore_ascii_case("in") {
                    op = "IN".to_string();
                } else {
                    op = raw.to_string();
                }
            }
            Rule::policy_value => {
                // policy_value = { "(" ~ string ~ ("," ~ string)* ~ ")" | string }
                for v in inner.into_inner() {
                    if v.as_rule() == Rule::string {
                        values.push(parse_string(v)?);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(PolicyConditionAst { field, op, values })
}

/// Parse attr_pair_list from an alter_user_set / alter_coll_set pair.
fn parse_attr_pair_list(pair: Pair<Rule>) -> Result<Vec<(String, String)>, ParseError> {
    let mut attributes = Vec::new();

    // The pair is alter_user_set or alter_coll_set, which contains kw_SET + attr_pair_list
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::attr_pair_list {
            for attr_pair in inner.into_inner() {
                if attr_pair.as_rule() == Rule::attr_pair {
                    let mut key = String::new();
                    let mut val = String::new();
                    for p in attr_pair.into_inner() {
                        match p.as_rule() {
                            Rule::ident => key = super::super::parse_ident(p),
                            Rule::string => val = parse_string(p)?,
                            _ => {}
                        }
                    }
                    attributes.push((key, val));
                }
            }
        }
    }

    Ok(attributes)
}

/// Parse ident_list from an alter_user_remove / alter_coll_remove pair.
fn parse_ident_list(pair: Pair<Rule>) -> Result<Vec<String>, ParseError> {
    let mut names = Vec::new();

    // The pair is alter_user_remove or alter_coll_remove, which contains kw_REMOVE + ident_list
    for inner in pair.into_inner() {
        if inner.as_rule() == Rule::ident_list {
            for ident in inner.into_inner() {
                if ident.as_rule() == Rule::ident {
                    names.push(super::super::parse_ident(ident));
                }
            }
        }
    }

    Ok(names)
}

#[cfg(test)]
mod tests {
    use crate::query::ast::Statement;
    use crate::query::ast::stmt::{AlterCollectionAttrsAst, AlterUserAst, PolicyEffectAst};
    use crate::query::parse::parse;

    #[test]
    fn test_parse_create_user() {
        let stmts = parse("CREATE USER 'alice' PASSWORD 'secret'").unwrap();
        match &stmts[0] {
            Statement::CreateUser(cu) => {
                assert_eq!(cu.username, "alice");
                assert_eq!(cu.password, "secret");
            }
            other => panic!("expected CreateUser, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_drop_user() {
        let stmts = parse("DROP USER 'alice'").unwrap();
        match &stmts[0] {
            Statement::DropUser(du) => {
                assert_eq!(du.username, "alice");
            }
            other => panic!("expected DropUser, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_user_password() {
        let stmts = parse("ALTER USER 'alice' PASSWORD 'new_pass'").unwrap();
        match &stmts[0] {
            Statement::AlterUser(au) => match au {
                AlterUserAst::Password { username, password } => {
                    assert_eq!(username, "alice");
                    assert_eq!(password, "new_pass");
                }
                other => panic!("expected Password variant, got {:?}", other),
            },
            other => panic!("expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_user_set() {
        let stmts = parse("ALTER USER 'alice' SET role = 'admin', dept = 'eng'").unwrap();
        match &stmts[0] {
            Statement::AlterUser(au) => match au {
                AlterUserAst::Set {
                    username,
                    attributes,
                } => {
                    assert_eq!(username, "alice");
                    assert_eq!(attributes.len(), 2);
                    assert_eq!(attributes[0], ("role".to_string(), "admin".to_string()));
                    assert_eq!(attributes[1], ("dept".to_string(), "eng".to_string()));
                }
                other => panic!("expected Set variant, got {:?}", other),
            },
            other => panic!("expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_user_remove() {
        let stmts = parse("ALTER USER 'alice' REMOVE role, dept").unwrap();
        match &stmts[0] {
            Statement::AlterUser(au) => match au {
                AlterUserAst::Remove {
                    username,
                    attributes,
                } => {
                    assert_eq!(username, "alice");
                    assert_eq!(attributes, &["role".to_string(), "dept".to_string()]);
                }
                other => panic!("expected Remove variant, got {:?}", other),
            },
            other => panic!("expected AlterUser, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_create_api_key() {
        let stmts = parse("CREATE API KEY 'my-key' FOR USER 'alice'").unwrap();
        match &stmts[0] {
            Statement::CreateApiKey(cak) => {
                assert_eq!(cak.name, "my-key");
                assert_eq!(cak.username, "alice");
            }
            other => panic!("expected CreateApiKey, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_drop_api_key() {
        let stmts = parse("DROP API KEY 'my-key'").unwrap();
        match &stmts[0] {
            Statement::DropApiKey(dak) => {
                assert_eq!(dak.name, "my-key");
            }
            other => panic!("expected DropApiKey, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_create_policy_simple() {
        let stmts = parse("CREATE POLICY admin_only WHEN action = 'write' ALLOW").unwrap();
        match &stmts[0] {
            Statement::CreatePolicy(cp) => {
                assert_eq!(cp.name, "admin_only");
                assert_eq!(cp.conditions.len(), 1);
                assert_eq!(cp.conditions[0].field, "action");
                assert_eq!(cp.conditions[0].op, "=");
                assert_eq!(cp.conditions[0].values, vec!["write".to_string()]);
                assert_eq!(cp.effect, PolicyEffectAst::Allow);
            }
            other => panic!("expected CreatePolicy, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_create_policy_multi_condition() {
        let stmts = parse(
            "CREATE POLICY dept_read WHEN subject.role = 'viewer' AND action IN ('read', 'list') DENY",
        )
        .unwrap();
        match &stmts[0] {
            Statement::CreatePolicy(cp) => {
                assert_eq!(cp.name, "dept_read");
                assert_eq!(cp.conditions.len(), 2);
                assert_eq!(cp.conditions[0].field, "subject.role");
                assert_eq!(cp.conditions[0].op, "=");
                assert_eq!(cp.conditions[0].values, vec!["viewer".to_string()]);
                assert_eq!(cp.conditions[1].field, "action");
                assert_eq!(cp.conditions[1].op, "IN");
                assert_eq!(
                    cp.conditions[1].values,
                    vec!["read".to_string(), "list".to_string()]
                );
                assert_eq!(cp.effect, PolicyEffectAst::Deny);
            }
            other => panic!("expected CreatePolicy, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_create_policy_not_equals() {
        let stmts = parse("CREATE POLICY non_admin WHEN subject.role != 'admin' DENY").unwrap();
        match &stmts[0] {
            Statement::CreatePolicy(cp) => {
                assert_eq!(cp.conditions[0].op, "!=");
            }
            other => panic!("expected CreatePolicy, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_drop_policy() {
        let stmts = parse("DROP POLICY admin_only").unwrap();
        match &stmts[0] {
            Statement::DropPolicy(dp) => {
                assert_eq!(dp.name, "admin_only");
            }
            other => panic!("expected DropPolicy, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_collection_set() {
        let stmts = parse("ALTER COLLECTION users SET ttl = '30d', access = 'public'").unwrap();
        match &stmts[0] {
            Statement::AlterCollectionAttrs(aca) => match aca {
                AlterCollectionAttrsAst::Set {
                    collection,
                    attributes,
                } => {
                    assert_eq!(collection, "users");
                    assert_eq!(attributes.len(), 2);
                    assert_eq!(attributes[0], ("ttl".to_string(), "30d".to_string()));
                    assert_eq!(attributes[1], ("access".to_string(), "public".to_string()));
                }
                other => panic!("expected Set variant, got {:?}", other),
            },
            other => panic!("expected AlterCollectionAttrs, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_alter_collection_remove() {
        let stmts = parse("ALTER COLLECTION users REMOVE ttl, access").unwrap();
        match &stmts[0] {
            Statement::AlterCollectionAttrs(aca) => match aca {
                AlterCollectionAttrsAst::Remove {
                    collection,
                    attributes,
                } => {
                    assert_eq!(collection, "users");
                    assert_eq!(attributes, &["ttl".to_string(), "access".to_string()]);
                }
                other => panic!("expected Remove variant, got {:?}", other),
            },
            other => panic!("expected AlterCollectionAttrs, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_case_insensitive() {
        // Auth statements should be case-insensitive
        assert!(parse("create user 'bob' password 'pass'").is_ok());
        assert!(parse("drop user 'bob'").is_ok());
        assert!(parse("Create Api Key 'k1' For User 'bob'").is_ok());
        assert!(parse("Drop Api Key 'k1'").is_ok());
        assert!(parse("create policy p1 when action = 'read' allow").is_ok());
        assert!(parse("drop policy p1").is_ok());
        assert!(parse("alter collection c1 set x = 'y'").is_ok());
        assert!(parse("alter collection c1 remove x").is_ok());
    }
}
