// SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0
// SPDX-FileCopyrightText: 2026 Mrinal Joshi and the Mercury Browser contributors

//! IPC across the confinement point. The child opens a channel to the parent before it confines
//! itself, receives a 4 MiB shared-memory payload on it after confinement, and opens a new
//! channel after confinement that works too. The child's exit code is the verdict.

#[path = "support/mod.rs"]
mod support;

use support::{Test, Tree};

const TESTS: &[Test] = &[(
    "ipc_after_entry_with_shared_memory",
    ipc_after_entry_with_shared_memory,
)];

const JOBS: &[Test] = &[("ipc", job_ipc)];

fn main() {
    support::run(TESTS, JOBS);
}

#[cfg(not(target_os = "macos"))]
fn ipc_after_entry_with_shared_memory() {}

#[cfg(target_os = "macos")]
fn ipc_after_entry_with_shared_memory() {
    use ipc_channel::ipc::{IpcOneShotServer, IpcSender, IpcSharedMemory};

    let tree = Tree::new("ipc", "home");
    let (server, name) =
        IpcOneShotServer::<IpcSender<Blob>>::new().expect("create the one-shot server");
    let (child, output) = support::spawn_child("ipc", &tree, &[("MERCURY_TEST_SERVER", &name)]);
    let (_, to_child) = server.accept().expect("accept the child's channel");
    let payload = vec![0xA5_u8; PAYLOAD];
    to_child
        .send(Blob(IpcSharedMemory::from_bytes(&payload)))
        .expect("send the payload to the child");
    support::expect_pass("ipc", &support::finish(child, &output));
}

#[cfg(target_os = "macos")]
use ipc_channel::ipc::IpcSharedMemory;

#[cfg(target_os = "macos")]
const PAYLOAD: usize = 4 * 1024 * 1024;

#[cfg(target_os = "macos")]
#[derive(serde::Serialize, serde::Deserialize)]
struct Blob(IpcSharedMemory);

#[cfg(target_os = "macos")]
fn job_ipc() {
    use ipc_channel::ipc::{self, IpcSender};

    let name = support::child_env("MERCURY_TEST_SERVER");
    // The channel to the parent is made before confinement, as the browser makes it.
    let (to_me, from_parent) = ipc::channel::<Blob>().expect("create the channel");
    let bootstrap = support::expect_ok(IpcSender::<IpcSender<Blob>>::connect(name), "connect");
    support::expect_ok(bootstrap.send(to_me), "send the channel to the parent");

    support::child_confine();

    let received = support::expect_ok(from_parent.recv(), "receive the payload");
    let bytes: &[u8] = &received.0;
    support::check(
        bytes.len() == PAYLOAD && bytes.iter().all(|byte| *byte == 0xA5),
        "the shared-memory payload changed in transit",
    );

    // A channel created after confinement carries a message to this same process.
    let (sender, receiver) = support::expect_ok(ipc::channel::<Blob>(), "create a channel");
    let blob = IpcSharedMemory::from_bytes(&bytes[..1024]);
    support::expect_ok(sender.send(Blob(blob)), "send on the new channel");
    let echoed = support::expect_ok(receiver.recv(), "receive on the new channel");
    support::check(
        echoed.0.len() == 1024,
        "the echoed payload has the wrong size",
    );
}
