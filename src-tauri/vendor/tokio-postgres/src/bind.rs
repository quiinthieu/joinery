use crate::client::InnerClient;
use crate::codec::FrontendMessage;
use crate::connection::RequestMessages;
use crate::prepare::get_type;
use crate::types::BorrowToSql;
use crate::{Column, Error, Portal, Statement, query};
use fallible_iterator::FallibleIterator;
use postgres_protocol::message::backend::Message;
use postgres_protocol::message::frontend;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

pub async fn bind<P, I>(
    client: &Arc<InnerClient>,
    statement: Statement,
    params: I,
) -> Result<Portal, Error>
where
    P: BorrowToSql,
    I: IntoIterator<Item = P>,
    I::IntoIter: ExactSizeIterator,
{
    let name = format!("p{}", NEXT_ID.fetch_add(1, Ordering::SeqCst));
    let buf = client.with_buf(|buf| {
        query::encode_bind(&statement, params, &name, buf)?;
        frontend::describe(b'P', &name, buf).map_err(Error::encode)?;
        frontend::sync(buf);
        Ok(buf.split().freeze())
    })?;

    // Own the server-side name before awaiting BindComplete, so cancellation
    // at any response boundary closes the portal without executing it.
    let mut portal = Portal::new(client, name, statement, Vec::new());
    let mut responses = client.send(RequestMessages::Single(FrontendMessage::Raw(buf)))?;

    match responses.next().await? {
        Message::BindComplete => {}
        _ => return Err(Error::unexpected_message()),
    }

    // Describe the bound portal, not the potentially stale prepared statement.
    // No Execute message has been sent.
    let mut columns = Vec::new();
    match responses.next().await? {
        Message::RowDescription(body) => {
            let mut fields = body.fields();
            while let Some(field) = fields.next().map_err(Error::parse)? {
                columns.push(Column {
                    name: field.name().to_owned(),
                    table_oid: Some(field.table_oid()).filter(|oid| *oid != 0),
                    column_id: Some(field.column_id()).filter(|id| *id != 0),
                    type_modifier: field.type_modifier(),
                    r#type: get_type(client, field.type_oid()).await?,
                });
            }
        }
        Message::NoData => {}
        _ => return Err(Error::unexpected_message()),
    }
    portal.set_columns(columns);
    Ok(portal)
}
