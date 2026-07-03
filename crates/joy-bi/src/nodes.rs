// Copyright (c) 2026 Joydev GmbH (joydev.com)
// SPDX-License-Identifier: LicenseRef-Commercial

//! The plugin result contract (Plugin-Konzept §2, JOY-01E8): the canonical
//! Rust shape of the node tree a plugin prints on stdout. The app mirrors
//! this in @joyint/plugin-schema; keep the `kind` tags and field names in
//! lockstep with that package.

use serde::Serialize;

/// Scalar leaf carried by data nodes. Bool/Null are part of the wire
/// contract even while no reference report emits them yet.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum Scalar {
    Text(String),
    Number(f64),
    Bool(bool),
    Null,
}

impl From<&str> for Scalar {
    fn from(v: &str) -> Self {
        Scalar::Text(v.to_string())
    }
}
impl From<String> for Scalar {
    fn from(v: String) -> Self {
        Scalar::Text(v)
    }
}
impl From<usize> for Scalar {
    fn from(v: usize) -> Self {
        Scalar::Number(v as f64)
    }
}
impl From<f64> for Scalar {
    fn from(v: f64) -> Self {
        Scalar::Number(v)
    }
}

/// Any node in the contract, tagged by `kind`. List/Text are contract
/// kinds the reference reports do not emit yet.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[allow(dead_code)]
pub enum Node {
    Value {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        value: Scalar,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    Table {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        columns: Vec<String>,
        rows: Vec<Vec<Scalar>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    List {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        items: Vec<Scalar>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
    Text {
        text: String,
    },
    Group {
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        children: Vec<Node>,
        #[serde(skip_serializing_if = "Option::is_none")]
        view: Option<String>,
    },
}

impl Node {
    pub fn value(label: &str, value: impl Into<Scalar>) -> Self {
        Node::Value {
            label: Some(label.to_string()),
            value: value.into(),
            unit: None,
            view: None,
        }
    }

    pub fn value_with_unit(label: &str, value: impl Into<Scalar>, unit: &str) -> Self {
        Node::Value {
            label: Some(label.to_string()),
            value: value.into(),
            unit: Some(unit.to_string()),
            view: None,
        }
    }

    pub fn table(label: &str, columns: &[&str], rows: Vec<Vec<Scalar>>, view: &str) -> Self {
        Node::Table {
            label: Some(label.to_string()),
            columns: columns.iter().map(|c| c.to_string()).collect(),
            rows,
            view: Some(view.to_string()),
        }
    }

    pub fn group(label: &str, children: Vec<Node>) -> Self {
        Node::Group {
            label: Some(label.to_string()),
            children,
            view: None,
        }
    }
}
