//! PipeWire part: keeps a picture of the audio graph and makes the links
//! to the friends' sink match what the user ticked.
//!
//! Links are created without `object.linger`, so they belong to this process:
//! dropping a `Link` proxy removes the link, and so does quitting or crashing.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use pipewire as pw;
use pw::link::Link;
use pw::node::{Node, NodeChangeMask, NodeListener};
use pw::types::ObjectType;

use crate::{Cmd, Entry, Shared, config::Config, tray::AppTray};

#[derive(Default)]
struct NodeData {
    name: String,
    class: String,
    app: Option<String>,
    /// loopback "cable" or other helper stream that should never be offered
    plumbing: bool,
}

struct PortData {
    node: u32,
    output: bool,
    channel: String,
}

struct State {
    cfg: Config,
    nodes: HashMap<u32, NodeData>,
    node_proxies: HashMap<u32, (Node, NodeListener)>,
    ports: HashMap<u32, PortData>,
    /// every link in the graph: link id -> (output port, input port)
    links: HashMap<u32, (u32, u32)>,
    /// links we created, keyed by (output port, input port); dropping one unlinks it
    ours: HashMap<(u32, u32), Link>,
    /// what the user ticked, by name ("sticky"); lives only in memory
    shared: BTreeSet<String>,
    tray: ksni::blocking::Handle<AppTray>,
    ui: Arc<Shared>,
}

impl State {
    /// Which entry a node belongs to, if it's something we can share.
    fn key_of(&self, n: &NodeData) -> Option<String> {
        // sinks from the config, offered under their label (their monitor is copied)
        if let Some(src) = self.cfg.sources.iter().find(|s| s.sink == n.name) {
            return Some(src.label.clone());
        }
        // app playback streams only; skip plumbing such as game-sink.output
        if n.class == "Stream/Output/Audio" && !n.plumbing {
            return Some(n.app.clone().unwrap_or_else(|| n.name.clone()));
        }
        None
    }

    /// Make the graph match `shared`: add missing links, remove unwanted ones.
    fn reconcile(&mut self, core: &pw::core::CoreRc) {
        // input ports of the friends' sink, by channel
        let target_id = self.cfg.target.as_ref().and_then(|t| {
            self.nodes
                .iter()
                .find(|(_, n)| &n.name == t)
                .map(|(id, _)| *id)
        });
        let mut target_in: BTreeMap<String, u32> = BTreeMap::new();
        if let Some(tid) = target_id {
            for (pid, p) in &self.ports {
                if p.node == tid && !p.output {
                    target_in.insert(p.channel.clone(), *pid);
                }
            }
        }

        // links we want: (out port, in port) -> (out node, in node)
        let mut want: HashMap<(u32, u32), (u32, u32)> = HashMap::new();
        if let Some(tid) = target_id {
            for (nid, n) in &self.nodes {
                if *nid == tid {
                    continue;
                }
                let Some(key) = self.key_of(n) else { continue };
                if !self.shared.contains(&key) {
                    continue;
                }
                for (pid, p) in &self.ports {
                    if p.node != *nid || !p.output {
                        continue;
                    }
                    if p.channel == "MONO" || target_in.len() == 1 {
                        // mono source → every target input; mono target ← every channel
                        for in_pid in target_in.values() {
                            want.insert((*pid, *in_pid), (*nid, tid));
                        }
                    } else if let Some(in_pid) = target_in.get(&p.channel) {
                        want.insert((*pid, *in_pid), (*nid, tid));
                    }
                }
            }
        }

        // remove ours that are no longer wanted (dropping the proxy destroys the link)
        self.ours.retain(|k, _| want.contains_key(k));

        // create missing ones (skip if that link already exists, e.g. drawn in qpwgraph)
        let existing: BTreeSet<(u32, u32)> = self.links.values().copied().collect();
        for ((out_p, in_p), (out_n, in_n)) in want {
            if self.ours.contains_key(&(out_p, in_p)) || existing.contains(&(out_p, in_p)) {
                continue;
            }
            let props = pw::properties::properties! {
                "link.output.port" => out_p.to_string(),
                "link.input.port" => in_p.to_string(),
                "link.output.node" => out_n.to_string(),
                "link.input.node" => in_n.to_string(),
            };
            match core.create_object::<Link>("link-factory", &props) {
                Ok(link) => {
                    self.ours.insert((out_p, in_p), link);
                }
                Err(e) => eprintln!("could not link {out_p} -> {in_p}: {e}"),
            }
        }

        self.publish();
    }

    /// Send the current list to the tray and the window.
    fn publish(&self) {
        let present: BTreeSet<String> =
            self.nodes.values().filter_map(|n| self.key_of(n)).collect();
        let mut entries: Vec<Entry> = present
            .union(&self.shared)
            .map(|k| Entry {
                key: k.clone(),
                shared: self.shared.contains(k),
                present: present.contains(k),
            })
            .collect();
        // configured sources first (in config order), then apps alphabetically
        let rank = |k: &str| {
            self.cfg
                .sources
                .iter()
                .position(|s| s.label == k)
                .unwrap_or(usize::MAX)
        };
        entries.sort_by_key(|e| (rank(&e.key), e.key.to_lowercase()));

        let status = match &self.cfg.target {
            None => Some("No target sink set in config".to_string()),
            Some(t) if !self.nodes.values().any(|n| &n.name == t) => {
                Some(format!("Target sink \"{t}\" not found"))
            }
            Some(_) => None,
        };

        *self.ui.entries.lock().unwrap() = entries.clone();
        *self.ui.status.lock().unwrap() = status.clone();
        if let Some(ctx) = self.ui.egui_ctx.lock().unwrap().as_ref() {
            ctx.request_repaint();
        }
        self.tray.update(move |t: &mut AppTray| {
            t.entries = entries;
            t.status = status;
        });
    }
}

/// Runs until `Cmd::Quit` (Ctrl+C / SIGTERM are handled in main.rs).
pub fn run(
    cfg: Config,
    rx: pw::channel::Receiver<Cmd>,
    tray: ksni::blocking::Handle<AppTray>,
    ui: Arc<Shared>,
) -> anyhow::Result<()> {
    pw::init();
    let main_loop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&main_loop, None)?;
    let core = context.connect_rc(None)?;
    let registry = core.get_registry_rc()?;

    let state = Rc::new(RefCell::new(State {
        cfg,
        nodes: HashMap::new(),
        node_proxies: HashMap::new(),
        ports: HashMap::new(),
        links: HashMap::new(),
        ours: HashMap::new(),
        shared: BTreeSet::new(),
        tray,
        ui,
    }));

    // ── commands from tray / window ──
    let _rx = {
        let state = state.clone();
        let core = core.clone();
        let ml = main_loop.clone();
        rx.attach(main_loop.loop_(), move |cmd| {
            let mut s = state.borrow_mut();
            match cmd {
                Cmd::Toggle(key) => {
                    if !s.shared.remove(&key) {
                        s.shared.insert(key);
                    }
                }
                Cmd::StopAll => s.shared.clear(),
                Cmd::Quit => {
                    s.shared.clear();
                    s.ours.clear(); // unlink everything we made
                    ml.quit();
                    return;
                }
            }
            s.reconcile(&core);
        })
    };

    // ── watch the graph ──
    let _reg_listener = {
        let state_g = state.clone();
        let state_r = state.clone();
        let core_g = core.clone();
        let core_r = core.clone();
        let registry_w = registry.downgrade();
        registry
            .add_listener_local()
            .global(move |obj| {
                let Some(props) = obj.props else { return };
                let mut s = state_g.borrow_mut();
                match obj.type_ {
                    ObjectType::Node => {
                        s.nodes.insert(
                            obj.id,
                            NodeData {
                                name: props.get("node.name").unwrap_or_default().to_string(),
                                class: props.get("media.class").unwrap_or_default().to_string(),
                                ..Default::default()
                            },
                        );
                        // bind to read the full properties (application.name, plumbing markers)
                        if let Some(registry) = registry_w.upgrade()
                            && let Ok(node) = registry.bind::<Node, _>(obj)
                        {
                            let id = obj.id;
                            let state_i = state_g.clone();
                            let core_i = core_g.clone();
                            let listener = node
                                .add_listener_local()
                                .info(move |info| {
                                    // Later info events (e.g. suspended → running) may carry
                                    // no properties; only read them when they changed.
                                    if !info.change_mask().contains(NodeChangeMask::PROPS) {
                                        return;
                                    }
                                    let Some(p) = info.props() else { return };
                                    let mut s = state_i.borrow_mut();
                                    if let Some(n) = s.nodes.get_mut(&id) {
                                        n.app = p.get("application.name").map(str::to_string);
                                        // loopbacks tag both halves with node.link-group;
                                        // node.passive may be "true", "out", "in-out", …
                                        let passive =
                                            p.get("node.passive").is_some_and(|v| v != "false");
                                        let linked = p.get("node.link-group").is_some();
                                        n.plumbing = passive || linked;
                                    }
                                    s.reconcile(&core_i);
                                })
                                .register();
                            s.node_proxies.insert(id, (node, listener));
                        }
                    }
                    ObjectType::Port => {
                        let (Some(node), Some(dir)) = (
                            props.get("node.id").and_then(|v| v.parse().ok()),
                            props.get("port.direction"),
                        ) else {
                            return;
                        };
                        s.ports.insert(
                            obj.id,
                            PortData {
                                node,
                                output: dir == "out",
                                channel: props.get("audio.channel").unwrap_or("MONO").to_string(),
                            },
                        );
                    }
                    ObjectType::Link => {
                        let (Some(o), Some(i)) = (
                            props.get("link.output.port").and_then(|v| v.parse().ok()),
                            props.get("link.input.port").and_then(|v| v.parse().ok()),
                        ) else {
                            return;
                        };
                        s.links.insert(obj.id, (o, i));
                        return; // a new link never requires new links
                    }
                    _ => return,
                }
                s.reconcile(&core_g);
            })
            .global_remove(move |id| {
                let mut s = state_r.borrow_mut();
                let was_node = s.nodes.remove(&id).is_some();
                let was_port = s.ports.remove(&id).is_some();
                s.node_proxies.remove(&id);
                s.links.remove(&id);
                // forget our link proxies whose ports are gone
                let ports: BTreeSet<u32> = s.ports.keys().copied().collect();
                s.ours
                    .retain(|(o, i), _| ports.contains(o) && ports.contains(i));
                if was_node || was_port {
                    s.reconcile(&core_r);
                }
            })
            .register()
    };

    main_loop.run();

    // leaving: drop our links explicitly; disconnecting would remove them anyway
    state.borrow_mut().ours.clear();
    Ok(())
}
