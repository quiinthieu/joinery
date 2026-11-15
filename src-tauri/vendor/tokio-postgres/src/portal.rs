use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::{Column, Statement};
use postgres_protocol::message::frontend;
use std::sync::{Arc, Weak};

struct Inner {
    client: Weak<InnerClient>,
    name: String,
    _statement: Statement,
    description: Statement,
}

impl Drop for Inner {
    fn drop(&mut self) {
        if let Some(client) = self.client.upgrade() {
            let buf = client.with_buf(|buf| {
                frontend::close(b'P', &self.name, buf).unwrap();
                frontend::sync(buf);
                buf.split().freeze()
            });
            let _ = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)));
        }
    }
}

/// A portal.
///
/// Portals can only be used with the connection that created them, and only exist for the duration of the transaction
/// in which they were created.
#[derive(Clone)]
pub struct Portal(Arc<Inner>);

impl Portal {
    pub(crate) fn new(
        client: &Arc<InnerClient>,
        name: String,
        statement: Statement,
        columns: Vec<Column>,
    ) -> Portal {
        Portal(Arc::new(Inner {
            client: Arc::downgrade(client),
            name,
            _statement: statement,
            description: Statement::unnamed(Vec::new(), columns),
        }))
    }

    pub(crate) fn set_columns(&mut self, columns: Vec<Column>) {
        // Called only during bind, before the portal can be cloned or returned.
        Arc::get_mut(&mut self.0)
            .expect("newly bound portal is uniquely owned")
            .description = Statement::unnamed(Vec::new(), columns);
    }

    pub(crate) fn name(&self) -> &str {
        &self.0.name
    }

    /// Returns metadata described from this already-bound portal.
    ///
    /// Binding and describing do not execute the portal. Inspect these columns
    /// before calling `Transaction::query_portal` to execute this same portal.
    pub fn columns(&self) -> &[Column] {
        self.0.description.columns()
    }

    pub(crate) fn statement(&self) -> &Statement {
        &self.0.description
    }
}
