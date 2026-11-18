use zbus::Connection;

/// Creates a session-bus connection intended to be shared by session-service
/// adapters. Call once at startup and pass cloned handles to adapters; do not
/// open one connection per widget or monitor. Later phases will integrate the bus driver
/// with the shell's event loop and evaluate disabling zbus' internal executor.
pub async fn connect_session() -> zbus::Result<Connection> {
    Connection::session().await
}

/// Creates a system-bus connection intended to be shared by system-service
/// adapters. UPower and other machine-level services live on this bus; call
/// once at startup and pass cloned handles to adapters.
pub async fn connect_system() -> zbus::Result<Connection> {
    Connection::system().await
}
