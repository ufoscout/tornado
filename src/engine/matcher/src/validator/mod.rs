pub mod id;

use crate::config::filter::Filter;
use crate::config::rule::Rule;
use crate::config::MatcherConfig;
use crate::error::MatcherError;
use log::*;
use std::collections::BTreeMap;

/// A validator for a MatcherConfig
#[derive(Default)]
pub struct MatcherConfigValidator {
    id: id::IdValidator,
}

impl MatcherConfigValidator {
    pub fn new() -> MatcherConfigValidator {
        MatcherConfigValidator { id: id::IdValidator::new() }
    }

    pub fn validate(&self, config: &MatcherConfig) -> Result<(), MatcherError> {
        match config {
            MatcherConfig::Rules { rules } => self.validate_rules(rules),
            MatcherConfig::Filter { filter, nodes } => self.validate_filter(filter, nodes),
        }
    }

    /// Validates that a Filter has a valid name and triggers the validation recursively
    /// for all filter's nodes.
    fn validate_filter(
        &self,
        filter: &Filter,
        nodes: &BTreeMap<String, MatcherConfig>,
    ) -> Result<(), MatcherError> {
        info!("MatcherConfigValidator validate_filter - validate filter [{}]", filter.name);

        self.id.validate_filter_name(&filter.name)?;

        for (node_name, node_config) in nodes {
            self.id.validate_node_name(node_name)?;
            self.validate(node_config)?;
        }

        Ok(())
    }

    /// Validates a set of Rules.
    /// In addition to the checks performed by the validate(rule) method,
    /// it verifies that rule names are unique.
    fn validate_rules(&self, rules: &[Rule]) -> Result<(), MatcherError> {
        info!("MatcherConfigValidator validate_all - validate all rules");

        let mut rule_names = vec![];

        for rule in rules {
            if rule.active {
                self.validate_rule(rule)?;
                MatcherConfigValidator::check_unique_name(&mut rule_names, &rule.name)?;
            }
        }

        Ok(())
    }

    /// Checks that a rule:
    /// - has a valid name
    /// - has valid extracted variable names
    /// - has valid action IDs
    fn validate_rule(&self, rule: &Rule) -> Result<(), MatcherError> {
        let rule_name = &rule.name;

        info!("MatcherConfigValidator validate - Validating rule: [{}]", rule_name);
        self.id.validate_rule_name(rule_name)?;

        for var_name in rule.constraint.with.keys() {
            self.id.validate_extracted_var_name(var_name, rule_name)?
        }

        for action in &rule.actions {
            self.id.validate_action_id(&action.id, rule_name)?
        }

        Ok(())
    }

    fn check_unique_name(rule_names: &mut Vec<String>, name: &str) -> Result<(), MatcherError> {
        let name_string = name.to_owned();
        debug!(
            "MatcherConfigValidator - Validating uniqueness of name for rule: [{}]",
            &name_string
        );
        if rule_names.contains(&name_string) {
            return Err(MatcherError::NotUniqueRuleNameError { name: name_string });
        }
        rule_names.push(name_string);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::rule::{Action, Constraint, Extractor, ExtractorRegex, Operator};
    use maplit::*;
    use std::collections::HashMap;
    use tornado_common_api::Value;

    #[test]
    fn should_validate_correct_rule() {
        // Arrange
        let rule = new_rule(
            "rule_name",
            Operator::Equal {
                first: Value::Text("1".to_owned()),
                second: Value::Text("1".to_owned()),
            },
        );

        // Act
        let result = MatcherConfigValidator::new().validate_rules(&vec![rule]);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn should_validate_correct_rules() {
        // Arrange
        let rule_1 = new_rule(
            "rule_name",
            Operator::Equal {
                first: Value::Text("1".to_owned()),
                second: Value::Text("1".to_owned()),
            },
        );

        let rule_2 = new_rule(
            "rule_name_2",
            Operator::Equal {
                first: Value::Text("1".to_owned()),
                second: Value::Text("1".to_owned()),
            },
        );

        // Act
        let result = MatcherConfigValidator::new().validate_rules(&vec![rule_1, rule_2]);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn should_fail_validation_if_empty_name() {
        // Arrange
        let rule_1 = new_rule(
            "",
            Operator::Equal {
                first: Value::Text("1".to_owned()),
                second: Value::Text("1".to_owned()),
            },
        );

        // Act
        let result = MatcherConfigValidator::new().validate_rules(&vec![rule_1]);

        // Assert
        assert!(result.is_err());
    }

    #[test]
    fn build_should_fail_if_not_unique_name() {
        // Arrange
        let op = Operator::Equal {
            first: Value::Text("1".to_owned()),
            second: Value::Text("1".to_owned()),
        };
        let rule_1 = new_rule("rule_name", op.clone());
        let rule_2 = new_rule("rule_name", op.clone());

        // Act
        let matcher = MatcherConfigValidator::new().validate_rules(&vec![rule_1, rule_2]);

        // Assert
        assert!(matcher.is_err());

        match matcher.err().unwrap() {
            MatcherError::NotUniqueRuleNameError { name } => assert_eq!("rule_name", name),
            _ => assert!(false),
        }
    }

    #[test]
    fn build_should_fail_if_empty_spaces_in_rule_name() {
        // Arrange
        let op = Operator::Equal {
            first: Value::Text("1".to_owned()),
            second: Value::Text("1".to_owned()),
        };
        let rule_1 = new_rule("rule name", op.clone());

        // Act
        let matcher = MatcherConfigValidator::new().validate_rules(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_name() {
        // Arrange
        let op = Operator::Equal {
            first: Value::Text("1".to_owned()),
            second: Value::Text("1".to_owned()),
        };
        let rule_1 = new_rule("rule.name", op.clone());

        // Act
        let matcher = MatcherConfigValidator::new().validate_rules(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_extracted_var_name() {
        // Arrange
        let op = Operator::Equal {
            first: Value::Text("1".to_owned()),
            second: Value::Text("1".to_owned()),
        };
        let mut rule_1 = new_rule("rule_name", op.clone());

        rule_1.constraint.with.insert(
            "var.with.dot".to_owned(),
            Extractor {
                from: String::from("${event.type}"),
                regex: ExtractorRegex { regex: String::from(r"[0-9]+"), group_match_idx: 0 },
            },
        );

        // Act
        let matcher = MatcherConfigValidator::new().validate_rules(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_action_id() {
        // Arrange
        let op = Operator::Equal {
            first: Value::Text("1".to_owned()),
            second: Value::Text("1".to_owned()),
        };
        let mut rule_1 = new_rule("rule_name", op.clone());

        rule_1.actions.push(Action {
            id: "id.with.dot.and.question.mark?".to_owned(),
            payload: HashMap::new(),
        });

        // Act
        let matcher = MatcherConfigValidator::new().validate_rules(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_wrong_filter_name() {
        // Arrange
        let filter = Filter {
            filter: None,
            name: "wrong.because.of.dots".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        // Act
        let matcher = MatcherConfigValidator::new().validate_filter(&filter, &btreemap![]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn should_validate_filter_name() {
        // Arrange
        let filter = Filter {
            filter: None,
            name: "good_name".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        // Act
        let matcher = MatcherConfigValidator::new().validate_filter(&filter, &btreemap![]);

        // Assert
        assert!(matcher.is_ok());
    }

    #[test]
    fn build_should_fail_if_wrong_node_name() {
        // Arrange
        let filter = Filter {
            filter: None,
            name: "good_names".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        let rules = MatcherConfig::Rules { rules: vec![] };
        let node_name = "wrong.name!".to_owned();

        // Act
        let matcher =
            MatcherConfigValidator::new().validate_filter(&filter, &btreemap![node_name => rules]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn should_validate_node_name() {
        // Arrange
        let filter = Filter {
            filter: None,
            name: "good_names".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        let rules = MatcherConfig::Rules { rules: vec![] };
        let node_name = "great_name".to_owned();

        // Act
        let matcher =
            MatcherConfigValidator::new().validate_filter(&filter, &btreemap![node_name => rules]);

        // Assert
        assert!(matcher.is_ok());
    }

    #[test]
    fn should_validate_a_config_recursively() {
        // Arrange
        let filter1 = Filter {
            filter: None,
            name: "good_name".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        let filter2 = filter1.clone();
        let rule_1 = new_rule("rule_name", None);

        let config = MatcherConfig::Filter {
            filter: filter1,
            nodes: btreemap!(
                "0".to_owned() => MatcherConfig::Filter {filter: filter2, nodes: btreemap!()},
                "1".to_owned() => MatcherConfig::Rules{ rules: vec![rule_1]}
            ),
        };

        // Act
        let matcher = MatcherConfigValidator::new().validate(&config);

        // Assert
        assert!(matcher.is_ok());
    }

    #[test]
    fn should_validate_a_config_recursively_and_fail_if_wrong_inner_rule_name() {
        // Arrange
        let filter1 = Filter {
            filter: None,
            name: "good_name".to_owned(),
            active: true,
            description: "".to_owned(),
        };

        let filter2 = filter1.clone();
        let rule_1 = new_rule("rule.name!", None);

        let config = MatcherConfig::Filter {
            filter: filter1,
            nodes: btreemap!(
                "0".to_owned() => MatcherConfig::Filter {filter: filter2, nodes: btreemap!()},
                "1".to_owned() => MatcherConfig::Rules{ rules: vec![rule_1]}
            ),
        };

        // Act
        let matcher = MatcherConfigValidator::new().validate(&config);

        // Assert
        assert!(matcher.is_err());
    }

    fn new_rule<O: Into<Option<Operator>>>(name: &str, operator: O) -> Rule {
        let constraint = Constraint { where_operator: operator.into(), with: HashMap::new() };

        Rule {
            name: name.to_owned(),
            do_continue: true,
            active: true,
            actions: vec![],
            description: "".to_owned(),
            constraint,
        }
    }
}
