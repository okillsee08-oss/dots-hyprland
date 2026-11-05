use zbus::Connection;

/// Creates the process-wide session-bus connection.
///
/// Service adapters should clone/share this connection; they must not open one
/// connection per widget or monitor. Later phases will integrate the bus driver
/// with the shell's event loop and evaluate disabling zbus' internal executor.
pub async fn connect_session() -> zbus::Result<Connection> {
    Connection::session().await
}
