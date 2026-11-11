//! In-memory observations for one closed plan. These best-effort creation/DDL
//! markers detect drift; they are not immutable Snowflake object identities.
//! The owner correlates callbacks with physical admission and phase advancement.

use super::{DATABASE, Operation, Phase, Plan, Presence, SCHEMA, SchemaCensus, StatementId};
use crate::{
    domain::{AppError, AppResult},
    drivers::{
        snowflake_browsing::{self as browsing, BrowseRelation},
        snowflake_metadata::LiveContext,
        snowflake_qualification_metadata::{MetadataSite, RelationObservation, SchemaObservation},
    },
    relation_inspection::{RelationAvailability, RelationTraits},
};
use std::collections::HashSet;

fn invalid() -> AppError {
    AppError::invalid("Existing-schema qualification lifecycle evidence drifted")
}

struct TargetPin {
    created: String,
    last_ddl: String,
    traits: RelationTraits,
}

pub(super) struct Lifecycle {
    table: String,
    operations: Vec<Operation>,
    census: Option<SchemaCensus>,
    context: Option<LiveContext>,
    target: Option<TargetPin>,
    post_add_observed: bool,
}

impl Lifecycle {
    pub(super) fn new(plan: &Plan) -> Self {
        Self {
            table: plan.table().to_owned(),
            operations: plan.phases().iter().map(Phase::operation).collect(),
            census: None,
            context: None,
            target: None,
            post_add_observed: false,
        }
    }

    pub(super) fn census(&self) -> Option<&SchemaCensus> {
        self.census.as_ref()
    }

    pub(super) fn context(&self) -> Option<&LiveContext> {
        self.context.as_ref()
    }

    pub(super) fn validate_ownership(&self, marker: &super::OwnershipMarker) -> AppResult<()> {
        let context = self.context.as_ref().ok_or_else(invalid)?;
        let target = self.target.as_ref().ok_or_else(invalid)?;
        if context.role.as_deref() != Some(marker.owner())
            || target.created != marker.created()
            || target.traits.transient != Some(marker.transient())
        {
            return Err(invalid());
        }
        Ok(())
    }

    pub(super) fn observe_schemas(
        &mut self,
        phase: &Phase,
        observation: SchemaObservation<'_>,
    ) -> AppResult<()> {
        self.validate_phase(phase)?;
        let valid_site = match observation.site {
            MetadataSite::CreateTargetCheck => matches!(
                phase.operation(),
                Operation::DdlPreflight(StatementId::Create)
                    | Operation::ReviewedDdl(StatementId::Create)
            ),
            MetadataSite::SessionContextOptions => matches!(
                phase.operation(),
                Operation::CoherentReconciliation(_) | Operation::SessionOptionsResume
            ),
            MetadataSite::RelationCatalogBatch => matches!(
                phase.operation(),
                Operation::CoherentReconciliation(_) | Operation::Catalog(_)
            ),
            _ => false,
        };
        if !valid_site || observation.wire_names != observation.parsed_names {
            return Err(invalid());
        }
        self.validate_context(observation.context)?;
        if let Some(census) = &self.census {
            census.verify_observation(observation.wire_names)?;
        } else {
            if phase.number() != 2
                || phase.operation() != Operation::DdlPreflight(StatementId::Create)
                || observation.site != MetadataSite::CreateTargetCheck
            {
                return Err(invalid());
            }
            let census = SchemaCensus::freeze(observation.wire_names)?;
            // Commit both pins only after all validation succeeds.
            self.census = Some(census);
            self.context = Some(observation.context.clone());
        }
        Ok(())
    }

    pub(super) fn observe_relations(
        &mut self,
        phase: &Phase,
        observation: RelationObservation<'_>,
    ) -> AppResult<()> {
        self.validate_phase(phase)?;
        let presence = match (observation.site, phase.operation()) {
            (
                MetadataSite::CreateTargetCheck,
                Operation::DdlPreflight(StatementId::Create)
                | Operation::ReviewedDdl(StatementId::Create),
            ) => Presence::FreshNameAbsent,
            (
                MetadataSite::ExactRelationCheck,
                Operation::Inspect(_)
                | Operation::DdlPreflight(StatementId::Add | StatementId::Drop)
                | Operation::ReviewedDdl(StatementId::Add | StatementId::Drop),
            ) => Presence::Present,
            (
                MetadataSite::RelationCatalogBatch | MetadataSite::RelationCatalogSnapshot,
                Operation::Catalog(presence) | Operation::CoherentReconciliation(presence),
            ) => presence,
            _ => return Err(invalid()),
        };
        let census = self.census.as_ref().ok_or_else(invalid)?;
        self.validate_context(observation.context)?;
        match observation.site {
            MetadataSite::RelationCatalogSnapshot if observation.schema.is_none() => {}
            MetadataSite::RelationCatalogBatch => {
                let schema = observation.schema.ok_or_else(invalid)?;
                if !census.names().iter().any(|name| name == schema) {
                    return Err(invalid());
                }
            }
            MetadataSite::CreateTargetCheck | MetadataSite::ExactRelationCheck
                if observation.schema == Some(SCHEMA) => {}
            _ => return Err(invalid()),
        }
        if observation.relations.len() > browsing::RELATION_LIMIT {
            return Err(invalid());
        }
        let mut identities = HashSet::new();
        let mut target = None;
        for relation in observation.relations {
            if relation.database != DATABASE
                || !census.names().contains(&relation.table.schema)
                || observation
                    .schema
                    .is_some_and(|schema| relation.table.schema != schema)
                || !identities.insert((
                    relation.database.as_str(),
                    relation.table.schema.as_str(),
                    relation.table.name.as_str(),
                ))
            {
                return Err(invalid());
            }
            if relation.table.schema == SCHEMA && relation.table.name == self.table {
                target = Some(relation);
            }
        }
        // Exact reads are an observation of this target alone, not another row
        // that happens to have a compatible type or creation marker.
        if observation.site == MetadataSite::ExactRelationCheck
            && (observation.relations.len() != 1 || target.is_none())
        {
            return Err(invalid());
        }
        let covers_target = observation.schema.is_none() || observation.schema == Some(SCHEMA);
        match presence {
            Presence::FreshNameAbsent | Presence::DroppedNameAbsent if target.is_some() => {
                return Err(invalid());
            }
            Presence::Present if covers_target && target.is_none() => return Err(invalid()),
            _ => {}
        }
        if let Some(target) = target {
            self.observe_target(phase, target)?;
        }
        Ok(())
    }

    fn validate_phase(&self, phase: &Phase) -> AppResult<()> {
        let index = phase.number().checked_sub(1).ok_or_else(invalid)? as usize;
        if self.operations.get(index) != Some(&phase.operation()) {
            return Err(invalid());
        }
        Ok(())
    }

    fn validate_context(&self, context: &LiveContext) -> AppResult<()> {
        if context.database.as_deref() != Some(DATABASE)
            || context.schema.as_deref() != Some(SCHEMA)
            || context.quoted_identifiers_ignore_case
            || context.principal.is_empty()
            || context.session_id.is_empty()
            || self.context.as_ref().is_some_and(|pin| pin != context)
        {
            return Err(invalid());
        }
        // Bound the retained complete context using the production serializer.
        context.fingerprint()?;
        Ok(())
    }

    fn observe_target(&mut self, phase: &Phase, relation: &BrowseRelation) -> AppResult<()> {
        if relation.database != DATABASE
            || relation.table.schema != SCHEMA
            || relation.table.name != self.table
            || relation.table_type != "BASE TABLE"
            || relation.availability != RelationAvailability::Available
            || relation.traits.transient.is_none()
            || relation.traits.temporary != Some(false)
            || relation.traits.iceberg != Some(false)
            || relation.traits.dynamic != Some(false)
            || relation.traits.hybrid != Some(false)
            || relation.traits.immutable != Some(false)
        {
            return Err(invalid());
        }
        let created = marker(relation.created.as_deref())?;
        let last_ddl = marker(relation.last_ddl.as_deref())?;
        if let Some(pin) = &mut self.target {
            if pin.created != created || pin.traits != relation.traits {
                return Err(invalid());
            }
            if phase.number() == 10 && !self.post_add_observed {
                // The allowance is consumed even if Snowflake reports the same
                // timestamp. Subsequent batches/snapshots may not drift again.
                pin.last_ddl = last_ddl.to_owned();
                self.post_add_observed = true;
            } else if pin.last_ddl != last_ddl {
                return Err(invalid());
            }
        } else {
            if phase.number() != 4
                || phase.operation() != Operation::CoherentReconciliation(Presence::Present)
            {
                return Err(invalid());
            }
            self.target = Some(TargetPin {
                created: created.to_owned(),
                last_ddl: last_ddl.to_owned(),
                traits: relation.traits.clone(),
            });
        }
        Ok(())
    }
}

fn marker(value: Option<&str>) -> AppResult<&str> {
    value
        .filter(|value| {
            !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        })
        .ok_or_else(invalid)
}

#[cfg(test)]
#[path = "snowflake_table_qualification_lifecycle_tests.rs"]
mod tests;
