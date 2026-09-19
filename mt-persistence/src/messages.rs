//! Chat history: one row per sent or received text message.
//!
//! Unlike the other entities, messages have client-only state (delivery
//! status), so they get a real column layout. The originating `MeshPacket`
//! is still stored as a blob for diagnostics and future use.

use meshtastic_protobufs::meshtastic::{MeshPacket, PortNum, mesh_packet};
use rusqlite::{OptionalExtension, Row, params};

use crate::schema::encode;
use crate::{Database, Result, now_unix};

/// Client-side delivery state of an outbound message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageStatus {
    /// Persisted, waiting for the transport (or a reconnect).
    Queued,
    /// Handed to the radio / seen in the device queue.
    Enroute,
    /// An acknowledgement arrived.
    Delivered,
    /// Failed to deliver (no route, timeout, NAK, ...).
    Failed,
}

impl MessageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageStatus::Queued => "queued",
            MessageStatus::Enroute => "enroute",
            MessageStatus::Delivered => "delivered",
            MessageStatus::Failed => "failed",
        }
    }

    /// Parse a stored value; unknown strings become `Failed` so the UI
    /// never shows a message as being in flight forever.
    pub fn parse(value: &str) -> Self {
        match value {
            "queued" => MessageStatus::Queued,
            "enroute" => MessageStatus::Enroute,
            "delivered" => MessageStatus::Delivered,
            _ => MessageStatus::Failed,
        }
    }

    /// Whether no further status transition is expected.
    pub fn is_terminal(self) -> bool {
        matches!(self, MessageStatus::Delivered | MessageStatus::Failed)
    }
}

/// A chat message as stored in the database.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageRecord {
    /// Local autoincrement id (stable ordering key).
    pub id: i64,
    /// On-wire packet id.
    pub packet_id: u32,
    pub channel: u32,
    pub from: u32,
    pub to: u32,
    pub portnum: i32,
    pub text: String,
    /// Unix seconds the message was (or is scheduled to be) transmitted.
    pub sent_at: i64,
    /// Unix seconds the row was written locally.
    pub received_at: i64,
    pub status: MessageStatus,
    pub outgoing: bool,
    pub want_ack: bool,
    /// For reactions: the id of the message being reacted to.
    pub reply_id: u32,
    pub rx_snr: Option<f32>,
    pub rx_rssi: Option<i32>,
    pub hop_start: Option<u32>,
    pub hop_limit: Option<u32>,
    /// Failure reason, for outgoing messages that failed.
    pub error: Option<String>,
}

/// Which slice of the history to read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageFilter {
    /// Everything (diagnostics / export).
    All,
    /// The broadcast timeline of one channel.
    Channel(u32),
    /// A direct conversation between `my_num` and `peer`.
    Peer { my_num: u32, peer: u32 },
}

/// Paging parameters for [`Database::list_messages`].
#[derive(Debug, Clone)]
pub struct MessageQuery {
    pub filter: MessageFilter,
    pub limit: u32,
    /// Return only rows with an id below this value (backwards paging).
    pub before_id: Option<i64>,
}

impl MessageQuery {
    pub fn channel(channel: u32, limit: u32) -> Self {
        Self {
            filter: MessageFilter::Channel(channel),
            limit,
            before_id: None,
        }
    }

    pub fn peer(my_num: u32, peer: u32, limit: u32) -> Self {
        Self {
            filter: MessageFilter::Peer { my_num, peer },
            limit,
            before_id: None,
        }
    }

    pub fn before(mut self, id: i64) -> Self {
        self.before_id = Some(id);
        self
    }
}

/// Human-readable text of a packet, empty for non-text port numbers.
fn packet_text(packet: &MeshPacket) -> String {
    match &packet.payload_variant {
        Some(mesh_packet::PayloadVariant::Decoded(data)) => match PortNum::try_from(data.portnum) {
            Ok(PortNum::TextMessageApp) | Ok(PortNum::AlertApp) => {
                String::from_utf8_lossy(&data.payload).into_owned()
            }
            _ => String::new(),
        },
        _ => String::new(),
    }
}

fn packet_portnum(packet: &MeshPacket) -> i32 {
    match &packet.payload_variant {
        Some(mesh_packet::PayloadVariant::Decoded(data)) => data.portnum,
        _ => PortNum::UnknownApp as i32,
    }
}

fn packet_reply_id(packet: &MeshPacket) -> u32 {
    match &packet.payload_variant {
        Some(mesh_packet::PayloadVariant::Decoded(data)) => data.reply_id,
        _ => 0,
    }
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<MessageRecord> {
    let status: String = row.get("status")?;
    Ok(MessageRecord {
        id: row.get("id")?,
        packet_id: row.get("packet_id")?,
        channel: row.get("channel")?,
        from: row.get("from_num")?,
        to: row.get("to_num")?,
        portnum: row.get("portnum")?,
        text: row.get("text")?,
        sent_at: row.get("sent_at")?,
        received_at: row.get("received_at")?,
        status: MessageStatus::parse(&status),
        outgoing: row.get::<_, i64>("is_outgoing")? != 0,
        want_ack: row.get::<_, i64>("want_ack")? != 0,
        reply_id: row.get("reply_id")?,
        rx_snr: row.get("rx_snr")?,
        rx_rssi: row.get("rx_rssi")?,
        hop_start: row.get("hop_start")?,
        hop_limit: row.get("hop_limit")?,
        error: row.get("error")?,
    })
}

const SELECT_COLUMNS: &str = "id, packet_id, channel, from_num, to_num, portnum, text, \
     sent_at, received_at, status, is_outgoing, want_ack, reply_id, \
     rx_snr, rx_rssi, hop_start, hop_limit, error";

impl Database {
    /// Persist a message.
    ///
    /// Returns the local row id. Re-inserting the same `(packet_id,
    /// outgoing)` pair is a no-op for received messages (relayed duplicates
    /// collapse) and refreshes the status for outgoing ones.
    pub fn insert_message(
        &self,
        packet: &MeshPacket,
        outgoing: bool,
        status: MessageStatus,
        sent_at: Option<i64>,
    ) -> Result<i64> {
        let text = packet_text(packet);
        let portnum = packet_portnum(packet);
        let reply_id = packet_reply_id(packet);
        let sent_at = sent_at
            .or_else(|| (packet.rx_time != 0).then_some(packet.rx_time as i64))
            .unwrap_or_else(now_unix);
        let received_at = now_unix();
        let rx_snr = (packet.rx_snr != 0.0).then_some(packet.rx_snr);
        let rx_rssi = (packet.rx_rssi != 0).then_some(packet.rx_rssi);
        let hop_start = (packet.hop_start != 0).then_some(packet.hop_start);
        let hop_limit = (packet.hop_limit != 0).then_some(packet.hop_limit);
        let blob = encode(packet);
        let status_str = status.as_str();

        self.with_conn(|conn| {
            let inserted = conn.execute(
                "INSERT OR IGNORE INTO messages (
                     packet_id, channel, from_num, to_num, portnum, text,
                     sent_at, received_at, status, is_outgoing, want_ack, reply_id,
                     rx_snr, rx_rssi, hop_start, hop_limit, error, packet_blob
                 ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                params![
                    packet.id,
                    packet.channel,
                    packet.from,
                    packet.to,
                    portnum,
                    text,
                    sent_at,
                    received_at,
                    status_str,
                    outgoing,
                    packet.want_ack,
                    reply_id,
                    rx_snr,
                    rx_rssi,
                    hop_start,
                    hop_limit,
                    Option::<String>::None,
                    blob,
                ],
            )?;

            // An outgoing message may be retried (e.g. after a reconnect):
            // refresh its delivery metadata rather than dropping the update.
            if inserted == 0 && outgoing {
                conn.execute(
                    "UPDATE messages
                        SET status = ?1, sent_at = ?2, received_at = ?3,
                            hop_start = COALESCE(?4, hop_start),
                            hop_limit = COALESCE(?5, hop_limit)
                      WHERE packet_id = ?6 AND is_outgoing = 1",
                    params![
                        status_str,
                        sent_at,
                        received_at,
                        hop_start,
                        hop_limit,
                        packet.id
                    ],
                )?;
            }

            let id = conn.query_row(
                "SELECT id FROM messages WHERE packet_id = ?1 AND is_outgoing = ?2",
                params![packet.id, outgoing],
                |row| row.get::<_, i64>(0),
            )?;
            Ok(id)
        })
    }

    /// Fetch a stored message row by its wire identity.
    pub fn find_message(&self, packet_id: u32, outgoing: bool) -> Result<Option<MessageRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SELECT_COLUMNS} FROM messages
                  WHERE packet_id = ?1 AND is_outgoing = ?2"
            );
            Ok(conn
                .query_row(&sql, params![packet_id, outgoing], row_to_record)
                .optional()?)
        })
    }

    /// Fetch the original `MeshPacket` for a stored message, if its blob is
    /// still present. Used to faithfully retransmit queued messages.
    pub fn find_message_packet(
        &self,
        packet_id: u32,
        outgoing: bool,
    ) -> Result<Option<MeshPacket>> {
        self.with_conn(|conn| {
            let blob = conn
                .query_row(
                    "SELECT packet_blob FROM messages
                      WHERE packet_id = ?1 AND is_outgoing = ?2 AND packet_blob IS NOT NULL",
                    params![packet_id, outgoing],
                    |row| row.get::<_, Option<Vec<u8>>>(0),
                )
                .optional()?
                .flatten();
            match blob {
                Some(bytes) => Ok(Some(crate::schema::decode(&bytes)?)),
                None => Ok(None),
            }
        })
    }

    /// Update the delivery status of an outgoing message.
    pub fn mark_message_status(
        &self,
        packet_id: u32,
        outgoing: bool,
        status: MessageStatus,
        error: Option<&str>,
    ) -> Result<usize> {
        self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE messages SET status = ?1, error = ?2
                  WHERE packet_id = ?3 AND is_outgoing = ?4",
                params![status.as_str(), error, packet_id, outgoing],
            )?)
        })
    }

    /// Return in-flight outgoing messages to `Queued` (used when a link
    /// drops so they can be retried on the next connection).
    pub fn requeue_inflight(&self) -> Result<usize> {
        self.with_conn(|conn| {
            Ok(conn.execute(
                "UPDATE messages SET status = 'queued'
                  WHERE is_outgoing = 1 AND status = 'enroute'",
                [],
            )?)
        })
    }

    /// Outgoing messages still awaiting delivery, oldest first.
    pub fn pending_outgoing(&self) -> Result<Vec<MessageRecord>> {
        self.with_conn(|conn| {
            let sql = format!(
                "SELECT {SELECT_COLUMNS} FROM messages
                  WHERE is_outgoing = 1 AND status IN ('queued', 'enroute')
                  ORDER BY id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], row_to_record)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(Into::into)
        })
    }

    /// Read a page of chat history, newest page first but returned in
    /// chronological order for direct rendering.
    pub fn list_messages(&self, query: &MessageQuery) -> Result<Vec<MessageRecord>> {
        let limit = query.limit.clamp(1, 5_000) as i64;
        self.with_conn(|conn| {
            let (where_clause, bound): (String, Vec<Box<dyn rusqlite::ToSql>>) = match &query.filter
            {
                MessageFilter::All => ("1 = 1".to_string(), vec![]),
                MessageFilter::Channel(channel) => (
                    "(channel = ?1 AND to_num = 4294967295)".to_string(),
                    vec![Box::new(*channel)],
                ),
                MessageFilter::Peer { my_num, peer } => (
                    "((from_num = ?1 AND to_num = ?2) OR (from_num = ?2 AND to_num = ?1))"
                        .to_string(),
                    vec![Box::new(*my_num), Box::new(*peer)],
                ),
            };

            let mut sql = format!("SELECT {SELECT_COLUMNS} FROM messages WHERE {where_clause}");
            let mut params_dyn: Vec<Box<dyn rusqlite::ToSql>> = bound;
            if let Some(before) = query.before_id {
                sql.push_str(&format!(" AND id < ?{}", params_dyn.len() + 1));
                params_dyn.push(Box::new(before));
            }
            sql.push_str(&format!(" ORDER BY id DESC LIMIT {limit}"));

            let mut stmt = conn.prepare(&sql)?;
            let params_ref: Vec<&dyn rusqlite::ToSql> =
                params_dyn.iter().map(|b| b.as_ref()).collect();
            let rows = stmt.query_map(params_ref.as_slice(), row_to_record)?;
            let mut records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            // Query walked backwards for paging; hand back oldest -> newest.
            records.reverse();
            Ok(records)
        })
    }

    /// Total number of stored messages (optionally for one filter).
    pub fn message_count(&self, filter: &MessageFilter) -> Result<u32> {
        self.with_conn(|conn| {
            let count: u32 = match filter {
                MessageFilter::All => {
                    conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?
                }
                MessageFilter::Channel(channel) => conn.query_row(
                    "SELECT COUNT(*) FROM messages WHERE channel = ?1 AND to_num = 4294967295",
                    params![channel],
                    |r| r.get(0),
                )?,
                MessageFilter::Peer { my_num, peer } => conn.query_row(
                    "SELECT COUNT(*) FROM messages
                      WHERE (from_num = ?1 AND to_num = ?2) OR (from_num = ?2 AND to_num = ?1)",
                    params![my_num, peer],
                    |r| r.get(0),
                )?,
            };
            Ok(count)
        })
    }

    /// Delete one message by local id.
    pub fn delete_message(&self, id: i64) -> Result<usize> {
        self.with_conn(|conn| Ok(conn.execute("DELETE FROM messages WHERE id = ?1", params![id])?))
    }

    /// Forget a whole conversation (used by "clear chat").
    pub fn clear_conversation(&self, filter: &MessageFilter) -> Result<usize> {
        self.with_conn(|conn| {
            let changed = match filter {
                MessageFilter::All => conn.execute("DELETE FROM messages", [])?,
                MessageFilter::Channel(channel) => conn.execute(
                    "DELETE FROM messages WHERE channel = ?1 AND to_num = 4294967295",
                    params![channel],
                )?,
                MessageFilter::Peer { my_num, peer } => conn.execute(
                    "DELETE FROM messages
                      WHERE (from_num = ?1 AND to_num = ?2) OR (from_num = ?2 AND to_num = ?1)",
                    params![my_num, peer],
                )?,
            };
            Ok(changed)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use meshtastic_protobufs::meshtastic::Data;

    fn text_packet(id: u32, from: u32, to: u32, channel: u32, text: &str) -> MeshPacket {
        MeshPacket {
            from,
            to,
            channel,
            id,
            rx_time: 1_700_000_000,
            rx_snr: -7.0,
            hop_start: 3,
            hop_limit: 2,
            want_ack: true,
            payload_variant: Some(mesh_packet::PayloadVariant::Decoded(Data {
                portnum: PortNum::TextMessageApp as i32,
                payload: text.as_bytes().to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }
    }

    const BROADCAST: u32 = u32::MAX;
    const ME: u32 = 0xAAAA_0001;
    const PEER: u32 = 0xBBBB_0002;

    #[test]
    fn outgoing_status_lifecycle() {
        let db = Database::open_in_memory().unwrap();
        let packet = text_packet(10, ME, BROADCAST, 0, "hello mesh");
        let id = db
            .insert_message(&packet, true, MessageStatus::Queued, Some(1_700_000_000))
            .unwrap();
        assert!(id > 0);

        assert_eq!(db.pending_outgoing().unwrap().len(), 1);
        db.mark_message_status(10, true, MessageStatus::Enroute, None)
            .unwrap();
        assert_eq!(db.pending_outgoing().unwrap().len(), 1);
        db.mark_message_status(10, true, MessageStatus::Delivered, None)
            .unwrap();
        assert!(db.pending_outgoing().unwrap().is_empty());

        let msgs = db.list_messages(&MessageQuery::channel(0, 50)).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "hello mesh");
        assert!(msgs[0].outgoing);
        assert_eq!(msgs[0].status, MessageStatus::Delivered);
    }

    #[test]
    fn received_relay_duplicates_collapse() {
        let db = Database::open_in_memory().unwrap();
        let packet = text_packet(42, PEER, BROADCAST, 0, "hi");
        let a = db
            .insert_message(&packet, false, MessageStatus::Delivered, None)
            .unwrap();
        let b = db
            .insert_message(&packet, false, MessageStatus::Delivered, None)
            .unwrap();
        assert_eq!(a, b);
        assert_eq!(db.message_count(&MessageFilter::All).unwrap(), 1);
    }

    #[test]
    fn channel_and_peer_views_are_separate() {
        let db = Database::open_in_memory().unwrap();
        // Channel 0 broadcast.
        db.insert_message(
            &text_packet(1, PEER, BROADCAST, 0, "channel hi"),
            false,
            MessageStatus::Delivered,
            None,
        )
        .unwrap();
        // Direct message channel 0 (legacy) - must show in the peer thread only.
        db.insert_message(
            &text_packet(2, PEER, ME, 0, "direct hi"),
            false,
            MessageStatus::Delivered,
            None,
        )
        .unwrap();
        // Our reply.
        db.insert_message(
            &text_packet(3, ME, PEER, 0, "direct reply"),
            true,
            MessageStatus::Delivered,
            None,
        )
        .unwrap();
        // A different channel broadcast.
        db.insert_message(
            &text_packet(4, PEER, BROADCAST, 2, "ch2"),
            false,
            MessageStatus::Delivered,
            None,
        )
        .unwrap();

        assert_eq!(
            db.list_messages(&MessageQuery::channel(0, 50))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            db.list_messages(&MessageQuery::channel(2, 50))
                .unwrap()
                .len(),
            1
        );
        let thread = db.list_messages(&MessageQuery::peer(ME, PEER, 50)).unwrap();
        assert_eq!(thread.len(), 2);
        assert_eq!(thread[0].text, "direct hi");
        assert_eq!(thread[1].text, "direct reply");
    }

    #[test]
    fn before_id_pages_backwards() {
        let db = Database::open_in_memory().unwrap();
        for i in 0..10u32 {
            db.insert_message(
                &text_packet(100 + i, PEER, BROADCAST, 0, &format!("m{i}")),
                false,
                MessageStatus::Delivered,
                None,
            )
            .unwrap();
        }
        let page = db.list_messages(&MessageQuery::channel(0, 4)).unwrap();
        assert_eq!(page.len(), 4);
        assert_eq!(page.last().unwrap().text, "m9");
        let older = db
            .list_messages(&MessageQuery::channel(0, 4).before(page[0].id))
            .unwrap();
        assert_eq!(older.len(), 4);
        assert_eq!(older.last().unwrap().text, "m5");
    }

    #[test]
    fn requeue_inflight_returns_messages_to_queue() {
        let db = Database::open_in_memory().unwrap();
        db.insert_message(
            &text_packet(1, ME, BROADCAST, 0, "a"),
            true,
            MessageStatus::Enroute,
            None,
        )
        .unwrap();
        db.insert_message(
            &text_packet(2, ME, BROADCAST, 0, "b"),
            true,
            MessageStatus::Delivered,
            None,
        )
        .unwrap();
        assert_eq!(db.requeue_inflight().unwrap(), 1);
        let pending = db.pending_outgoing().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].text, "a");
    }

    #[test]
    fn non_text_packets_store_empty_text() {
        let db = Database::open_in_memory().unwrap();
        let mut packet = text_packet(5, PEER, BROADCAST, 0, "");
        if let Some(mesh_packet::PayloadVariant::Decoded(data)) = &mut packet.payload_variant {
            data.portnum = PortNum::TelemetryApp as i32;
            data.payload = vec![0x01, 0x02];
        }
        db.insert_message(&packet, false, MessageStatus::Delivered, None)
            .unwrap();
        assert!(
            db.list_messages(&MessageQuery::channel(0, 10)).unwrap()[0]
                .text
                .is_empty()
        );
    }
}
