pub mod id;

use crate::config::Rule;
use crate::error::MatcherError;
use log::*;
use std::collections::HashMap;

/// A validator for a Rule or array of Rules
#[derive(Default)]
pub struct RuleValidator {
    id: id::IdValidator,
}

impl RuleValidator {
    pub fn new() -> RuleValidator {
        RuleValidator { id: id::IdValidator::new() }
    }

    /// Checks that a rule:
    /// - has a valid name
    /// - has valid extracted variable names
    /// - has valid action IDs
    pub fn validate(&self, rule: &Rule) -> Result<(), MatcherError> {
        let rule_name = &rule.name;

        info!("RuleValidator validate - Validating rule: [{}]", rule_name);
        self.id.validate_rule_name(rule_name)?;

        for var_name in rule.constraint.with.keys() {
            self.id.validate_extracted_var_name(var_name, rule_name)?
        }

        for action in &rule.actions {
            self.id.validate_action_id(&action.id, rule_name)?
        }

        Ok(())
    }

    /// Validates a set of Rules.
    /// In addition to the checks performed by the validate(rule) method,
    ///   it verifies that rule names and rule priorities are unique.
    pub fn validate_all(&self, rules: &[Rule]) -> Result<(), MatcherError> {
        info!("RuleValidator validate_all - validate all rules");

        let mut rule_names = vec![];
        let mut rules_by_priority = HashMap::new();

        for rule in rules {
            if rule.active {
                self.validate(rule)?;
                RuleValidator::check_unique_name(&mut rule_names, &rule.name)?;
                RuleValidator::check_unique_priority(&mut rules_by_priority, &rule)?;
            }
        }

        Ok(())
    }

    fn check_unique_name(rule_names: &mut Vec<String>, name: &str) -> Result<(), MatcherError> {
        let name_string = name.to_owned();
        debug!("RuleValidator - Validating uniqueness of name for rule: [{}]", &name_string);
        if rule_names.contains(&name_string) {
            return Err(MatcherError::NotUniqueRuleNameError { name: name_string });
        }
        rule_names.push(name_string);
        Ok(())
    }

    fn check_unique_priority(
        rules_by_priority: &mut HashMap<u16, String>,
        rule: &Rule,
    ) -> Result<(), MatcherError> {
        debug!(
            "RuleValidator - Validating uniqueness of priority for rule: [{}] with priority [{}]",
            &rule.name, &rule.priority
        );
        if rules_by_priority.contains_key(&rule.priority) {
            return Err(MatcherError::NotUniqueRulePriorityError {
                first_rule_name: rules_by_priority[&rule.priority].to_owned(),
                second_rule_name: rule.name.to_owned(),
                priority: rule.priority,
            });
        }
        rules_by_priority.insert(rule.priority, rule.name.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::config::{Action, Constraint, Extractor, ExtractorRegex, Operator};
    use std::collections::HashMap;

    #[test]
    fn should_validate_correct_rule() {
        // Arrange
        let rule = new_rule(
            "rule_name",
            0,
            Operator::Equal { first: "1".to_owned(), second: "1".to_owned() },
        );

        // Act
        let result = RuleValidator::new().validate_all(&vec![rule]);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn should_validate_correct_rules() {
        // Arrange
        let rule_1 = new_rule(
            "rule_name",
            0,
            Operator::Equal { first: "1".to_owned(), second: "1".to_owned() },
        );

        let rule_2 = new_rule(
            "rule_name_2",
            1,
            Operator::Equal { first: "1".to_owned(), second: "1".to_owned() },
        );

        // Act
        let result = RuleValidator::new().validate_all(&vec![rule_1, rule_2]);

        // Assert
        assert!(result.is_ok());
    }

    #[test]
    fn build_should_fail_if_not_unique_name() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let rule_1 = new_rule("rule_name", 0, op.clone());
        let rule_2 = new_rule("rule_name", 1, op.clone());

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1, rule_2]);

        // Assert
        assert!(matcher.is_err());

        match matcher.err().unwrap() {
            MatcherError::NotUniqueRuleNameError { name } => assert_eq!("rule_name", name),
            _ => assert!(false),
        }
    }

    #[test]
    fn build_should_fail_if_not_unique_priority() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let rule_1 = new_rule("rule_1", 1, op.clone());
        let rule_2 = new_rule("rule_2", 1, op.clone());

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1, rule_2]);

        // Assert
        assert!(matcher.is_err());

        match matcher.err().unwrap() {
            MatcherError::NotUniqueRulePriorityError {
                first_rule_name,
                second_rule_name,
                priority,
            } => {
                assert_eq!("rule_1", first_rule_name);
                assert_eq!("rule_2", second_rule_name);
                assert_eq!(1, priority);
            }
            _ => assert!(false),
        }
    }

    #[test]
    fn build_should_fail_if_empty_spaces_in_rule_name() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let rule_1 = new_rule("rule name", 0, op.clone());

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_name() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let rule_1 = new_rule("rule.name", 0, op.clone());

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_extracted_var_name() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let mut rule_1 = new_rule("rule_name", 0, op.clone());

        rule_1.constraint.with.insert(
            "var.with.dot".to_owned(),
            Extractor {
                from: String::from("${event.type}"),
                regex: ExtractorRegex { regex: String::from(r"[0-9]+"), group_match_idx: 0 },
            },
        );

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    #[test]
    fn build_should_fail_if_not_correct_action_id() {
        // Arrange
        let op = Operator::Equal { first: "1".to_owned(), second: "1".to_owned() };
        let mut rule_1 = new_rule("rule_name", 0, op.clone());

        rule_1.actions.push(Action {
            id: "id.with.dot.and.question.mark?".to_owned(),
            payload: HashMap::new(),
        });

        // Act
        let matcher = RuleValidator::new().validate_all(&vec![rule_1]);

        // Assert
        assert!(matcher.is_err());
    }

    fn new_rule(name: &str, priority: u16, operator: Operator) -> Rule {
        let constraint = Constraint { where_operator: Some(operator), with: HashMap::new() };

        Rule {
            name: name.to_owned(),
            priority,
            do_continue: true,
            active: true,
            actions: vec![],
            description: "".to_owned(),
            constraint,
        }
    }
}
