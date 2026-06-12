//! GeneGIS Core — project model, layer graph, command bus, provenance.
//!
//! UI, AI, and CLI all emit [`Command`]s. The core stays independent of UX and AI.

pub mod command;
pub mod layer;
pub mod project;
pub mod provenance;
pub mod source;
pub mod view;
pub mod workspace;

pub use command::{Command, CommandBus, CommandEnvelope, CommandOrigin};
pub use layer::{Layer, LayerId, LayerKind, LayerStatistics};
pub use project::{Project, ProjectManifest};
pub use provenance::{ProvenanceEntry, ProvenanceStore};
pub use source::{DataSource, SourceKind};
pub use view::{View, ViewId, ViewKind};
pub use workspace::Workspace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_roundtrip_json() {
        let mut project = Project::new("demo");
        let source = DataSource::new("wards", SourceKind::File, "file:///data/wards.geojson");
        let source_id = source.id;
        project.workspace_mut().add_source(source);
        project.workspace_mut().add_layer(Layer::new(
            "Wards",
            LayerKind::Vector,
            source_id,
        ));

        let json = serde_json::to_string(&project).expect("serialize");
        let restored: Project = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.workspace().layers.len(), 1);
    }

    #[test]
    fn command_bus_records_history() {
        let mut bus = CommandBus::default();
        bus.push(CommandEnvelope::new(CommandOrigin::Cli, Command::Undo));
        assert_eq!(bus.history().len(), 1);
        assert!(bus.can_undo());
    }
}
