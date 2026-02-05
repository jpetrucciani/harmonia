#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EcosystemId {
    Python,
    Rust,
    Node,
    Go,
    Java,
    Custom(String),
}

pub mod custom;
pub mod go;
pub mod node;
pub mod python;
pub mod rust;
pub mod traits;

pub fn plugin_for(id: &EcosystemId) -> Box<dyn traits::EcosystemPlugin> {
    match id {
        EcosystemId::Python => Box::new(python::PythonPlugin),
        EcosystemId::Rust => Box::new(rust::RustPlugin),
        EcosystemId::Node => Box::new(node::NodePlugin),
        EcosystemId::Go => Box::new(go::GoPlugin),
        EcosystemId::Java => Box::new(custom::CustomPlugin),
        EcosystemId::Custom(_) => Box::new(custom::CustomPlugin),
    }
}
