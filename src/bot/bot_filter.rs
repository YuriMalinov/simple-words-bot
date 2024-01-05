use std::collections::HashMap;

use crate::model::{FilterValue, Task};

#[derive(Debug, PartialEq)]
pub(super) struct FilterGroup {
    values: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub(super) struct Filter {
    groups: Vec<FilterGroup>,
}

#[derive(Debug, PartialEq)]
pub(super) struct FilterInfo {
    pub(super) name: String,
    pub(super) possible_values: Vec<String>,
}

pub(super) trait HasFilterValues {
    fn get_filter_values(&self) -> &[FilterValue];
}

impl HasFilterValues for Task {
    fn get_filter_values(&self) -> &[FilterValue] {
        &self.filters
    }
}

pub(super) fn collect_filter_info(tasks: &[impl HasFilterValues]) -> Vec<FilterInfo> {
    let mut filters = HashMap::new();
    for task in tasks {
        for FilterValue { name, value } in task.get_filter_values() {
            let filter_group = filters
                .entry(name)
                .or_insert_with(HashMap::<String, &str>::new);
            filter_group.insert(value.to_lowercase(), value);
        }
    }

    let mut result = Vec::new();
    for (name, values) in filters {
        let mut possible_values = values.values().cloned().collect::<Vec<_>>();
        possible_values.sort();
        result.push(FilterInfo {
            name: name.clone(),
            possible_values: possible_values.into_iter().map(|s| s.into()).collect(),
        });
    }
    result.sort_by_key(|filter_info| filter_info.name.clone());
    result
}

pub(super) fn match_task(values: &[FilterValue], filter: &Filter) -> bool {
    for filter_group in &filter.groups {
        if !match_filter_group(values, filter_group) {
            return false;
        }
    }
    true
}

fn match_filter_group(values: &[FilterValue], filter_group: &FilterGroup) -> bool {
    for value in &filter_group.values {
        if values.iter().any(|filter| filter.value.contains(value)) {
            return true;
        }
    }
    false
}

pub(super) fn parse_filter(filter: &str) -> Filter {
    let mut groups = Vec::new();
    for group in filter.split(';') {
        let mut values = Vec::new();
        for value in group.split(',') {
            values.push(value.trim().to_lowercase());
        }
        groups.push(FilterGroup { values });
    }
    Filter { groups }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_parse_filter() {
        let filter = parse_filter("a, b ,c; d,e,f");
        assert_eq!(
            filter.groups,
            vec![
                FilterGroup {
                    values: vec!["a".into(), "b".into(), "c".into()]
                },
                FilterGroup {
                    values: vec!["d".into(), "e".into(), "f".into()]
                }
            ]
        );
    }

    struct TestTask {
        filters: Vec<FilterValue>,
    }

    impl HasFilterValues for TestTask {
        fn get_filter_values(&self) -> &[FilterValue] {
            &self.filters
        }
    }

    #[test]
    fn test_collect_filter_info() {
        let tasks = vec![
            TestTask {
                filters: vec![
                    FilterValue {
                        name: "test".into(),
                        value: "a".into(),
                    },
                    FilterValue {
                        name: "test".into(),
                        value: "b".into(),
                    },
                ],
            },
            TestTask {
                filters: vec![
                    FilterValue {
                        name: "test".into(),
                        value: "a".into(),
                    },
                    FilterValue {
                        name: "test".into(),
                        value: "c".into(),
                    },
                ],
            },
            TestTask {
                filters: vec![
                    FilterValue {
                        name: "test".into(),
                        value: "b".into(),
                    },
                    FilterValue {
                        name: "test".into(),
                        value: "c".into(),
                    },
                ],
            },
        ];

        let filter_info = collect_filter_info(&tasks);
        assert_eq!(
            filter_info,
            vec![FilterInfo {
                name: "test".into(),
                possible_values: vec!["a".into(), "b".into(), "c".into()]
            }]
        );
    }

    #[test]
    fn test_match() {
        let filter = parse_filter("a,b,c; d,e,f");
        assert!(match_task(
            &[
                FilterValue {
                    name: "test".into(),
                    value: "a".into()
                },
                FilterValue {
                    name: "test".into(),
                    value: "d".into()
                }
            ],
            &filter
        ));

        assert!(!match_task(
            &[FilterValue {
                name: "test".into(),
                value: "a".into()
            }],
            &filter
        ));
    }
}
