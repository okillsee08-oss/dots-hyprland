use zbus::Connection;

/// Creates the process-wide session-bus connection.
///
/// Service adapters should clone/share this connection; they must not open one
/// connection per widget or monitor. Later phases will integrate the bus driver
/// with the shell's event loop and evaluate disabling zbus' internal executor.
pub async fn connect_session() -> zbus::Result<Connection> {
    Connection::session().await
}


/// Creates the process-wide system-bus connection.
///
/// UPower and other machine-level services live on the system bus. Pass clones
/// of this connection to their adapters; do not open one connection per widget.
pub async fn connect_system() -> zbus::Result<Connection> {
    Connection::system().await
}
