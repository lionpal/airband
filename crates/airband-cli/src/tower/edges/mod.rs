//! What the tower reaches outside itself through: the model that reads, the processes on this computer, the
//! folders on its disk, and the ways his words get to an agent. Each is a trait the rest of the tower asks,
//! with the real one here and a stand-in in tests, so what the tower decides can be tried without any of them.
pub(super) mod deliver;
pub(super) mod folders;
pub(super) mod models;
pub(super) mod procs;

/// What an edge hands back when it has to wait for the world.
pub(super) type Boxed<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
