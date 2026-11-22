//! What the tower reaches outside itself through: the model that reads, the processes on this computer, the
//! folders on its disk, and the ways his words get to an agent. Each is a trait with the real one here; the
//! model, the processes and the folders also have a stand-in in tests. The ways to an agent are tried against
//! real sockets and commands in the end-to-end suites.
pub(super) mod deliver;
pub(super) mod folders;
pub(super) mod models;
pub(super) mod procs;

/// What an edge hands back when it has to wait for the world.
pub(super) type Boxed<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
