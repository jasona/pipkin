//! Chord's transport-independent service wire grammar: `{ serviceId, instance?, member, args }`
//! calls, the `$chord.service` control vocabulary, catalogues, subscription snapshots and
//! updates, the per-subscription Delta codec registry, and the consumer-side replica.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use crate::delta::{self, Decoder, Encoder, Op, WireOp};
use crate::error::{Error, Result, validation};

const SERVICE_CONTROL_ID: &str = "$chord.service";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Singleton,
    Keyed,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Singleton => "singleton",
            Mode::Keyed => "keyed",
        }
    }

    fn parse(value: &Value) -> Option<Mode> {
        match value.as_str() {
            Some("singleton") => Some(Mode::Singleton),
            Some("keyed") => Some(Mode::Keyed),
            _ => None,
        }
    }
}

/// A keyed service instance: its key and the generation of that key's current incarnation.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Address {
    pub key: String,
    pub generation: u64,
}

impl Address {
    fn to_value(&self) -> Value {
        json!({ "key": self.key, "generation": self.generation })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceCall {
    pub service_id: String,
    pub instance: Option<Address>,
    pub member: String,
    pub args: Vec<Value>,
}

impl ServiceCall {
    pub fn new(service_id: &str, member: &str, args: Vec<Value>) -> Self {
        ServiceCall {
            service_id: service_id.into(),
            instance: None,
            member: member.into(),
            args,
        }
    }

    pub fn to_value(&self) -> Value {
        let mut call = Map::new();
        call.insert("serviceId".into(), self.service_id.clone().into());
        if let Some(instance) = &self.instance {
            call.insert("instance".into(), instance.to_value());
        }
        call.insert("member".into(), self.member.clone().into());
        call.insert("args".into(), Value::Array(self.args.clone()));
        Value::Object(call)
    }
}

pub fn catalogue_call() -> ServiceCall {
    ServiceCall::new(SERVICE_CONTROL_ID, "catalogue", vec![])
}

pub fn subscribe_call(subscription_id: &str, service_id: &str, mode: Mode) -> ServiceCall {
    ServiceCall::new(
        SERVICE_CONTROL_ID,
        "subscribe",
        vec![
            subscription_id.into(),
            service_id.into(),
            mode.as_str().into(),
        ],
    )
}

pub fn unsubscribe_call(subscription_id: &str) -> ServiceCall {
    ServiceCall::new(
        SERVICE_CONTROL_ID,
        "unsubscribe",
        vec![subscription_id.into()],
    )
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControlCall {
    Catalogue,
    Subscribe {
        subscription_id: String,
        service_id: String,
        mode: Mode,
    },
    Unsubscribe {
        subscription_id: String,
    },
}

/// Recognize a `$chord.service` control call (used by test servers).
pub fn decode_control_call(call: &ServiceCall) -> Option<ControlCall> {
    if call.service_id != SERVICE_CONTROL_ID || call.instance.is_some() {
        return None;
    }
    let id = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    match (call.member.as_str(), call.args.as_slice()) {
        ("catalogue", []) => Some(ControlCall::Catalogue),
        ("subscribe", [sub, service, mode]) => Some(ControlCall::Subscribe {
            subscription_id: id(sub)?,
            service_id: id(service)?,
            mode: Mode::parse(mode)?,
        }),
        ("unsubscribe", [sub]) => Some(ControlCall::Unsubscribe {
            subscription_id: id(sub)?,
        }),
        _ => None,
    }
}

pub fn parse_service_call(value: &Value) -> Result<ServiceCall> {
    let map = value
        .as_object()
        .ok_or_else(|| validation("Invalid service call"))?;
    let allowed = ["serviceId", "member", "args", "instance"];
    if map.keys().any(|k| !allowed.contains(&k.as_str()))
        || !["serviceId", "member", "args"]
            .iter()
            .all(|k| map.contains_key(*k))
    {
        return Err(validation("Invalid service call"));
    }
    let id = |key: &str| {
        map[key]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| validation("Invalid service call"))
    };
    Ok(ServiceCall {
        service_id: id("serviceId")?,
        member: id("member")?,
        args: map["args"]
            .as_array()
            .cloned()
            .ok_or_else(|| validation("Invalid service call"))?,
        instance: map.get("instance").map(parse_address).transpose()?,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub service_id: String,
    pub mode: Mode,
}

pub fn parse_catalogue(value: &Value) -> Result<Vec<CatalogueEntry>> {
    let bad = || validation("Invalid service catalogue");
    let entries = value.as_array().ok_or_else(bad)?;
    let mut out: Vec<CatalogueEntry> = Vec::new();
    for entry in entries {
        let map = entry.as_object().ok_or_else(bad)?;
        if map.len() != 2 {
            return Err(bad());
        }
        let service_id = map
            .get("serviceId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(bad)?;
        let mode = map.get("mode").and_then(Mode::parse).ok_or_else(bad)?;
        if out.iter().any(|e| e.service_id == service_id) {
            return Err(bad());
        }
        out.push(CatalogueEntry {
            service_id: service_id.into(),
            mode,
        });
    }
    Ok(out)
}

pub fn catalogue_value(entries: &[CatalogueEntry]) -> Value {
    Value::Array(
        entries
            .iter()
            .map(|e| json!({ "serviceId": e.service_id, "mode": e.mode.as_str() }))
            .collect(),
    )
}

// ----------------------------------------------------------------------------- snapshots

/// A member of a service instance. `O` is the op representation: `WireOp` between the wire
/// and the decoder, `Op` after decoding.
#[derive(Clone, Debug, PartialEq)]
pub enum MemberSnapshot<O> {
    Method {
        name: String,
    },
    State {
        name: String,
        sequence: u64,
        ops: Vec<O>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstanceSnapshot<O> {
    pub instance: Option<Address>,
    pub members: Vec<MemberSnapshot<O>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubscriptionSnapshot<O> {
    pub service_id: String,
    pub mode: Mode,
    pub instances: Vec<InstanceSnapshot<O>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProviderUpdate<O> {
    State {
        instance: Option<Address>,
        member: String,
        sequence: u64,
        ops: Vec<O>,
    },
    Reset {
        snapshot: SubscriptionSnapshot<O>,
    },
    Unavailable,
    Replaced {
        snapshot: InstanceSnapshot<O>,
    },
    Spawned {
        instance: InstanceSnapshot<O>,
    },
    Closed {
        instance: Address,
    },
}

pub type WireSnapshot = SubscriptionSnapshot<WireOp>;
pub type WireUpdate = ProviderUpdate<WireOp>;
pub type Snapshot = SubscriptionSnapshot<Op>;
pub type Update = ProviderUpdate<Op>;

fn parse_address(value: &Value) -> Result<Address> {
    let bad = || validation("Invalid service instance address");
    let map = value.as_object().ok_or_else(bad)?;
    if map.len() != 2 {
        return Err(bad());
    }
    Ok(Address {
        key: map
            .get("key")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(bad)?
            .into(),
        generation: map
            .get("generation")
            .and_then(Value::as_u64)
            .filter(|g| *g >= 1)
            .ok_or_else(bad)?,
    })
}

/// Exactly `required` plus any subset of `optional`.
fn strict(
    map: &Map<String, Value>,
    required: &[&str],
    optional: &[&str],
    what: &str,
) -> Result<()> {
    let bad = || validation(format!("Invalid {what}"));
    if !required.iter().all(|k| map.contains_key(*k)) {
        return Err(bad());
    }
    if map
        .keys()
        .any(|k| !required.contains(&k.as_str()) && !optional.contains(&k.as_str()))
    {
        return Err(bad());
    }
    Ok(())
}

fn parse_instance_wire(value: &Value) -> Result<InstanceSnapshot<WireOp>> {
    let bad = || validation("Invalid service instance snapshot");
    let map = value.as_object().ok_or_else(bad)?;
    strict(
        map,
        &["members"],
        &["instance"],
        "service instance snapshot",
    )?;
    let instance = map.get("instance").map(parse_address).transpose()?;
    let mut members = Vec::new();
    for candidate in map["members"].as_array().ok_or_else(bad)? {
        let member = candidate
            .as_object()
            .ok_or_else(|| validation("Invalid service member snapshot"))?;
        let name = member
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty());
        match member.get("kind").and_then(Value::as_str) {
            Some("method") => {
                strict(member, &["name", "kind"], &[], "service method snapshot")?;
                members.push(MemberSnapshot::Method {
                    name: name
                        .ok_or_else(|| validation("Invalid service method snapshot"))?
                        .into(),
                });
            }
            Some("state") => {
                strict(
                    member,
                    &["name", "kind", "sequence", "ops"],
                    &[],
                    "service state snapshot",
                )?;
                let bad_state = || validation("Invalid service state snapshot");
                members.push(MemberSnapshot::State {
                    name: name.ok_or_else(bad_state)?.into(),
                    sequence: member["sequence"].as_u64().ok_or_else(bad_state)?,
                    ops: delta::parse_wire_ops(&member["ops"])?,
                });
            }
            _ => return Err(validation("Invalid service member snapshot")),
        }
    }
    Ok(InstanceSnapshot { instance, members })
}

pub fn parse_wire_snapshot(value: &Value) -> Result<WireSnapshot> {
    let bad = || validation("Invalid service subscription snapshot");
    let map = value.as_object().ok_or_else(bad)?;
    strict(
        map,
        &["serviceId", "mode", "instances"],
        &[],
        "service subscription snapshot",
    )?;
    Ok(SubscriptionSnapshot {
        service_id: map["serviceId"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(bad)?
            .into(),
        mode: Mode::parse(&map["mode"]).ok_or_else(bad)?,
        instances: map["instances"]
            .as_array()
            .ok_or_else(bad)?
            .iter()
            .map(parse_instance_wire)
            .collect::<Result<_>>()?,
    })
}

pub fn parse_wire_update(value: &Value) -> Result<WireUpdate> {
    let bad = || validation("Invalid service provider update");
    let map = value.as_object().ok_or_else(bad)?;
    match map.get("type").and_then(Value::as_str) {
        Some("state") => {
            strict(
                map,
                &["type", "member", "sequence", "ops"],
                &["instance"],
                "state update",
            )?;
            let bad_state = || validation("Invalid service state update");
            Ok(ProviderUpdate::State {
                instance: map.get("instance").map(parse_address).transpose()?,
                member: map["member"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(bad_state)?
                    .into(),
                sequence: map["sequence"]
                    .as_u64()
                    .filter(|s| *s >= 1)
                    .ok_or_else(bad_state)?,
                ops: delta::parse_wire_ops(&map["ops"])?,
            })
        }
        Some("reset") => {
            strict(map, &["type", "snapshot"], &[], "reset update")?;
            let snapshot = parse_wire_snapshot(&map["snapshot"])?;
            for instance in &snapshot.instances {
                for member in &instance.members {
                    if let MemberSnapshot::State { ops, .. } = member
                        && !matches!(ops.as_slice(), [WireOp::Replace(_)])
                    {
                        return Err(validation(
                            "Service reset must contain full root replacements",
                        ));
                    }
                }
            }
            Ok(ProviderUpdate::Reset { snapshot })
        }
        Some("unavailable") => {
            strict(map, &["type"], &[], "unavailable update")?;
            Ok(ProviderUpdate::Unavailable)
        }
        Some("replaced") => {
            strict(map, &["type", "snapshot"], &[], "replacement update")?;
            Ok(ProviderUpdate::Replaced {
                snapshot: parse_instance_wire(&map["snapshot"])?,
            })
        }
        Some("spawned") => {
            strict(map, &["type", "instance"], &[], "spawn update")?;
            Ok(ProviderUpdate::Spawned {
                instance: parse_instance_wire(&map["instance"])?,
            })
        }
        Some("closed") => {
            strict(map, &["type", "instance"], &[], "close update")?;
            Ok(ProviderUpdate::Closed {
                instance: parse_address(&map["instance"])?,
            })
        }
        _ => Err(bad()),
    }
}

fn instance_value(instance: &InstanceSnapshot<WireOp>) -> Value {
    let mut map = Map::new();
    if let Some(address) = &instance.instance {
        map.insert("instance".into(), address.to_value());
    }
    let members = instance
        .members
        .iter()
        .map(|m| match m {
            MemberSnapshot::Method { name } => json!({ "name": name, "kind": "method" }),
            MemberSnapshot::State {
                name,
                sequence,
                ops,
            } => json!({
                "name": name, "kind": "state", "sequence": sequence,
                "ops": ops.iter().map(delta::wire_op_value).collect::<Vec<_>>(),
            }),
        })
        .collect();
    map.insert("members".into(), Value::Array(members));
    Value::Object(map)
}

pub fn wire_snapshot_value(snapshot: &WireSnapshot) -> Value {
    json!({
        "serviceId": snapshot.service_id,
        "mode": snapshot.mode.as_str(),
        "instances": snapshot.instances.iter().map(instance_value).collect::<Vec<_>>(),
    })
}

pub fn wire_update_value(update: &WireUpdate) -> Value {
    match update {
        ProviderUpdate::State {
            instance,
            member,
            sequence,
            ops,
        } => {
            let mut v = json!({
                "type": "state", "member": member, "sequence": sequence,
                "ops": ops.iter().map(delta::wire_op_value).collect::<Vec<_>>(),
            });
            if let Some(address) = instance {
                v["instance"] = address.to_value();
            }
            v
        }
        ProviderUpdate::Reset { snapshot } => {
            json!({ "type": "reset", "snapshot": wire_snapshot_value(snapshot) })
        }
        ProviderUpdate::Unavailable => json!({ "type": "unavailable" }),
        ProviderUpdate::Replaced { snapshot } => {
            json!({ "type": "replaced", "snapshot": instance_value(snapshot) })
        }
        ProviderUpdate::Spawned { instance } => {
            json!({ "type": "spawned", "instance": instance_value(instance) })
        }
        ProviderUpdate::Closed { instance } => {
            json!({ "type": "closed", "instance": instance.to_value() })
        }
    }
}

// ----------------------------------------------------------------------------- codec registry

type StateKey = (Option<Address>, String);

struct Registry<C> {
    entries: HashMap<StateKey, C>,
}

impl<C: Default> Registry<C> {
    fn new() -> Self {
        Registry {
            entries: HashMap::new(),
        }
    }

    fn reset(&mut self) {
        self.entries.clear();
    }

    fn add(&mut self, instance: &Option<Address>, member: &str) -> Result<&mut C> {
        let key = (instance.clone(), member.to_owned());
        if self.entries.contains_key(&key) {
            return Err(Error::Delta(format!(
                "Duplicate service state {}",
                describe(instance, member)
            )));
        }
        Ok(self.entries.entry(key).or_default())
    }

    fn get(&mut self, instance: &Option<Address>, member: &str) -> Result<&mut C> {
        self.entries
            .get_mut(&(instance.clone(), member.to_owned()))
            .ok_or_else(|| {
                Error::Delta(format!(
                    "Unknown service state {}",
                    describe(instance, member)
                ))
            })
    }

    fn remove_instance(&mut self, instance: &Address) {
        self.entries
            .retain(|(i, _), _| i.as_ref() != Some(instance));
    }
}

fn describe(instance: &Option<Address>, member: &str) -> String {
    match instance {
        None => member.to_owned(),
        Some(a) => format!("{}@{}.{member}", a.key, a.generation),
    }
}

/// One Delta decoder per replicated state in ONE subscription. A fresh snapshot, a reset, a
/// replacement or unavailability restarts the path dictionaries, exactly as the provider does.
pub struct StateDecoder {
    registry: Registry<Decoder>,
}

impl Default for StateDecoder {
    fn default() -> Self {
        StateDecoder::new()
    }
}

impl StateDecoder {
    pub fn new() -> Self {
        StateDecoder {
            registry: Registry::new(),
        }
    }

    pub fn decode_snapshot(&mut self, snapshot: &WireSnapshot) -> Result<Snapshot> {
        self.registry.reset();
        Ok(SubscriptionSnapshot {
            service_id: snapshot.service_id.clone(),
            mode: snapshot.mode,
            instances: snapshot
                .instances
                .iter()
                .map(|i| self.decode_instance(i))
                .collect::<Result<_>>()?,
        })
    }

    pub fn decode_update(&mut self, update: &WireUpdate) -> Result<Update> {
        match update {
            ProviderUpdate::Reset { snapshot } => Ok(ProviderUpdate::Reset {
                snapshot: self.decode_snapshot(snapshot)?,
            }),
            ProviderUpdate::State {
                instance,
                member,
                sequence,
                ops,
            } => Ok(ProviderUpdate::State {
                instance: instance.clone(),
                member: member.clone(),
                sequence: *sequence,
                ops: self.registry.get(instance, member)?.decode(ops)?,
            }),
            ProviderUpdate::Replaced { snapshot } => {
                self.registry.reset();
                Ok(ProviderUpdate::Replaced {
                    snapshot: self.decode_instance(snapshot)?,
                })
            }
            ProviderUpdate::Spawned { instance } => Ok(ProviderUpdate::Spawned {
                instance: self.decode_instance(instance)?,
            }),
            ProviderUpdate::Unavailable => {
                self.registry.reset();
                Ok(ProviderUpdate::Unavailable)
            }
            ProviderUpdate::Closed { instance } => {
                self.registry.remove_instance(instance);
                Ok(ProviderUpdate::Closed {
                    instance: instance.clone(),
                })
            }
        }
    }

    fn decode_instance(
        &mut self,
        instance: &InstanceSnapshot<WireOp>,
    ) -> Result<InstanceSnapshot<Op>> {
        let mut members = Vec::with_capacity(instance.members.len());
        for member in &instance.members {
            members.push(match member {
                MemberSnapshot::Method { name } => MemberSnapshot::Method { name: name.clone() },
                MemberSnapshot::State {
                    name,
                    sequence,
                    ops,
                } => MemberSnapshot::State {
                    name: name.clone(),
                    sequence: *sequence,
                    ops: self.registry.add(&instance.instance, name)?.decode(ops)?,
                },
            });
        }
        Ok(InstanceSnapshot {
            instance: instance.instance.clone(),
            members,
        })
    }
}

/// The provider-side counterpart, for tests and the mock server.
pub struct StateEncoder {
    registry: Registry<Encoder>,
}

impl Default for StateEncoder {
    fn default() -> Self {
        StateEncoder::new()
    }
}

impl StateEncoder {
    pub fn new() -> Self {
        StateEncoder {
            registry: Registry::new(),
        }
    }

    pub fn encode_snapshot(&mut self, snapshot: &Snapshot) -> Result<WireSnapshot> {
        self.registry.reset();
        Ok(SubscriptionSnapshot {
            service_id: snapshot.service_id.clone(),
            mode: snapshot.mode,
            instances: snapshot
                .instances
                .iter()
                .map(|i| self.encode_instance(i))
                .collect::<Result<_>>()?,
        })
    }

    pub fn encode_update(&mut self, update: &Update) -> Result<WireUpdate> {
        match update {
            ProviderUpdate::Reset { snapshot } => Ok(ProviderUpdate::Reset {
                snapshot: self.encode_snapshot(snapshot)?,
            }),
            ProviderUpdate::State {
                instance,
                member,
                sequence,
                ops,
            } => Ok(ProviderUpdate::State {
                instance: instance.clone(),
                member: member.clone(),
                sequence: *sequence,
                ops: self.registry.get(instance, member)?.encode(ops),
            }),
            ProviderUpdate::Replaced { snapshot } => {
                self.registry.reset();
                Ok(ProviderUpdate::Replaced {
                    snapshot: self.encode_instance(snapshot)?,
                })
            }
            ProviderUpdate::Spawned { instance } => Ok(ProviderUpdate::Spawned {
                instance: self.encode_instance(instance)?,
            }),
            ProviderUpdate::Unavailable => {
                self.registry.reset();
                Ok(ProviderUpdate::Unavailable)
            }
            ProviderUpdate::Closed { instance } => {
                self.registry.remove_instance(instance);
                Ok(ProviderUpdate::Closed {
                    instance: instance.clone(),
                })
            }
        }
    }

    fn encode_instance(
        &mut self,
        instance: &InstanceSnapshot<Op>,
    ) -> Result<InstanceSnapshot<WireOp>> {
        let mut members = Vec::with_capacity(instance.members.len());
        for member in &instance.members {
            members.push(match member {
                MemberSnapshot::Method { name } => MemberSnapshot::Method { name: name.clone() },
                MemberSnapshot::State {
                    name,
                    sequence,
                    ops,
                } => MemberSnapshot::State {
                    name: name.clone(),
                    sequence: *sequence,
                    ops: self.registry.add(&instance.instance, name)?.encode(ops),
                },
            });
        }
        Ok(InstanceSnapshot {
            instance: instance.instance.clone(),
            members,
        })
    }
}

// ----------------------------------------------------------------------------- replica

/// What one applied update changed, so adapters can react without diffing whole states.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    State {
        instance: Option<Address>,
        member: String,
    },
    /// Instances were replaced wholesale (reset, replacement, spawn or close).
    Instances,
    /// The provider went away; every state is unready until a replacement arrives.
    Unavailable,
    /// A keyed update for an instance generation this replica does not hold.
    Ignored,
}

#[derive(Clone, Debug, PartialEq)]
enum Member {
    Method,
    State { sequence: u64, value: Value },
}

#[derive(Clone, Debug)]
struct ReplicaInstance {
    address: Option<Address>,
    members: HashMap<String, Member>,
}

/// The consumer's copy of one service subscription. Any error invalidates it: the caller must
/// drop the subscription and resubscribe (a fresh snapshot), never patch around the failure.
#[derive(Debug)]
pub struct Replica {
    service_id: String,
    mode: Mode,
    instances: Vec<ReplicaInstance>,
    available: bool,
    failed: bool,
}

impl Replica {
    /// Install a hydration snapshot. Every state member must start from a base batch.
    pub fn hydrate(snapshot: Snapshot) -> Result<Replica> {
        let mut replica = Replica {
            service_id: snapshot.service_id,
            mode: snapshot.mode,
            instances: Vec::new(),
            available: true,
            failed: false,
        };
        for instance in snapshot.instances {
            let installed = Self::install(instance)?;
            replica.instances.push(installed);
        }
        Ok(replica)
    }

    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// False after `unavailable` until a replacement arrives, or after a failure.
    pub fn is_ready(&self) -> bool {
        self.available && !self.failed
    }

    /// The current value of a singleton service's state member.
    pub fn state(&self, member: &str) -> Option<&Value> {
        self.instance_state(&None, member)
    }

    pub fn instance_state(&self, instance: &Option<Address>, member: &str) -> Option<&Value> {
        if !self.is_ready() {
            return None;
        }
        let instance = self.instances.iter().find(|i| &i.address == instance)?;
        match instance.members.get(member)? {
            Member::State { value, .. } => Some(value),
            Member::Method => None,
        }
    }

    pub fn sequence(&self, instance: &Option<Address>, member: &str) -> Option<u64> {
        let instance = self.instances.iter().find(|i| &i.address == instance)?;
        match instance.members.get(member)? {
            Member::State { sequence, .. } => Some(*sequence),
            Member::Method => None,
        }
    }

    pub fn instances(&self) -> Vec<Option<Address>> {
        self.instances.iter().map(|i| i.address.clone()).collect()
    }

    fn install(snapshot: InstanceSnapshot<Op>) -> Result<ReplicaInstance> {
        let mut members = HashMap::new();
        for member in snapshot.members {
            let (name, installed) = match member {
                MemberSnapshot::Method { name } => (name, Member::Method),
                MemberSnapshot::State {
                    name,
                    sequence,
                    ops,
                } => {
                    if !delta::is_base(&ops) {
                        return Err(Error::Delta(
                            "Replicated state snapshot is not a base operation batch".into(),
                        ));
                    }
                    (
                        name,
                        Member::State {
                            sequence,
                            value: delta::apply(None, ops)?,
                        },
                    )
                }
            };
            if members.insert(name.clone(), installed).is_some() {
                return Err(validation(format!(
                    "Service instance repeats member {name:?}"
                )));
            }
        }
        Ok(ReplicaInstance {
            address: snapshot.instance,
            members,
        })
    }

    /// Apply one decoded update. Sequence gaps and unapplicable ops are errors and leave the
    /// replica failed.
    pub fn apply(&mut self, update: Update) -> Result<Change> {
        if self.failed {
            return Err(Error::Delta("replica has failed".into()));
        }
        let result = self.apply_inner(update);
        if result.is_err() {
            self.failed = true;
            self.instances.clear();
        }
        result
    }

    fn apply_inner(&mut self, update: Update) -> Result<Change> {
        match update {
            ProviderUpdate::State {
                instance,
                member,
                sequence,
                ops,
            } => {
                if !self.available {
                    return Err(Error::Delta(
                        "state update while the service is unavailable".into(),
                    ));
                }
                let Some(target) = self.instances.iter_mut().find(|i| i.address == instance) else {
                    return match self.mode {
                        // A keyed update for an incarnation we no longer hold is expected noise.
                        Mode::Keyed => Ok(Change::Ignored),
                        Mode::Singleton => {
                            Err(Error::Delta("state update for an unknown instance".into()))
                        }
                    };
                };
                let Some(Member::State {
                    sequence: current,
                    value,
                }) = target.members.get_mut(&member)
                else {
                    return Err(Error::Delta(format!(
                        "update for unknown state member {member:?}"
                    )));
                };
                if sequence != *current + 1 {
                    return Err(Error::SequenceGap {
                        expected: *current + 1,
                        got: sequence,
                    });
                }
                let previous = std::mem::take(value);
                *value = delta::apply(Some(previous), ops)?;
                *current = sequence;
                Ok(Change::State { instance, member })
            }
            ProviderUpdate::Reset { snapshot } => {
                if snapshot.service_id != self.service_id || snapshot.mode != self.mode {
                    return Err(validation(
                        "Remote service reset has the wrong service or mode",
                    ));
                }
                let mut instances = Vec::new();
                for instance in snapshot.instances {
                    instances.push(Self::install(instance)?);
                }
                self.instances = instances;
                self.available = true;
                Ok(Change::Instances)
            }
            ProviderUpdate::Unavailable => {
                self.available = false;
                self.instances.clear();
                Ok(Change::Unavailable)
            }
            ProviderUpdate::Replaced { snapshot } => {
                self.instances = vec![Self::install(snapshot)?];
                self.available = true;
                Ok(Change::Instances)
            }
            ProviderUpdate::Spawned { instance } => {
                let installed = Self::install(instance)?;
                self.instances.retain(|i| i.address != installed.address);
                self.instances.push(installed);
                Ok(Change::Instances)
            }
            ProviderUpdate::Closed { instance } => {
                self.instances
                    .retain(|i| i.address.as_ref() != Some(&instance));
                Ok(Change::Instances)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_member(name: &str, sequence: u64, ops: Vec<Op>) -> MemberSnapshot<Op> {
        MemberSnapshot::State {
            name: name.into(),
            sequence,
            ops,
        }
    }

    fn singleton(members: Vec<MemberSnapshot<Op>>) -> Snapshot {
        SubscriptionSnapshot {
            service_id: "pi.models".into(),
            mode: Mode::Singleton,
            instances: vec![InstanceSnapshot {
                instance: None,
                members,
            }],
        }
    }

    fn set(path: &str, value: Value) -> Op {
        Op::Set(vec![delta::Seg::Key(path.into())], value)
    }

    fn state_update(member: &str, sequence: u64, ops: Vec<Op>) -> Update {
        ProviderUpdate::State {
            instance: None,
            member: member.into(),
            sequence,
            ops,
        }
    }

    #[test]
    fn control_calls_round_trip_and_validate() {
        assert_eq!(
            decode_control_call(&catalogue_call()),
            Some(ControlCall::Catalogue)
        );
        let sub = subscribe_call("subscription-1", "pi.models", Mode::Singleton);
        assert_eq!(
            decode_control_call(&sub),
            Some(ControlCall::Subscribe {
                subscription_id: "subscription-1".into(),
                service_id: "pi.models".into(),
                mode: Mode::Singleton
            })
        );
        assert_eq!(
            decode_control_call(&unsubscribe_call("subscription-1")),
            Some(ControlCall::Unsubscribe {
                subscription_id: "subscription-1".into()
            })
        );
        assert_eq!(parse_service_call(&sub.to_value()).unwrap(), sub);
        let keyed = json!({
            "serviceId": "pi.question-dialog", "instance": { "key": "invocation-1", "generation": 2 },
            "member": "submit", "args": [{ "outcome": "selected", "index": 0 }],
        });
        assert_eq!(parse_service_call(&keyed).unwrap().member, "submit");
        assert!(
            parse_service_call(
                &json!({ "serviceId": "pi.models", "member": "list", "args": [], "extra": true })
            )
            .is_err()
        );
    }

    #[test]
    fn catalogue_validation() {
        let ok = parse_catalogue(&json!([
            { "serviceId": "pi.models", "mode": "singleton" },
            { "serviceId": "pi.dialogs", "mode": "keyed" },
        ]))
        .unwrap();
        assert_eq!(ok.len(), 2);
        assert_eq!(parse_catalogue(&catalogue_value(&ok)).unwrap(), ok);
        for bad in [
            json!([{ "serviceId": "pi.models", "mode": "unknown" }]),
            json!([{ "serviceId": "pi.models", "mode": "keyed" }, { "serviceId": "pi.models", "mode": "keyed" }]),
            json!([{ "serviceId": "pi.models", "mode": "keyed", "extra": 1 }]),
            json!([{ "serviceId": "", "mode": "keyed" }]),
            json!({}),
        ] {
            assert!(parse_catalogue(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn wire_updates_are_validated() {
        assert!(
            parse_wire_update(
                &json!({ "type": "state", "member": "state", "sequence": 0, "ops": [] })
            )
            .is_err()
        );
        assert!(
            parse_wire_update(
                &json!({ "type": "state", "member": "state", "sequence": 1, "ops": [["?", 0]] })
            )
            .is_err()
        );
        assert!(
            parse_wire_update(
                &json!({ "type": "state", "member": "state", "sequence": 1, "ops": [], "extra": 1 })
            )
            .is_err()
        );
        assert!(parse_wire_update(&json!({ "type": "reset", "snapshot": {} })).is_err());
        assert!(parse_wire_update(&json!({ "type": "unavailable", "extra": 1 })).is_err());
        assert!(
            parse_wire_update(
                &json!({ "type": "closed", "instance": { "key": "k", "generation": 0 } })
            )
            .is_err()
        );
        let not_root = json!({
            "type": "reset",
            "snapshot": { "serviceId": "s", "mode": "singleton", "instances": [
                { "members": [{ "name": "state", "kind": "state", "sequence": 103, "ops": [["s", ["before"], 103]] }] }
            ]},
        });
        assert!(
            parse_wire_update(&not_root)
                .unwrap_err()
                .to_string()
                .contains("full root replacements")
        );
    }

    #[test]
    fn one_codec_pair_per_subscription_state() {
        let mut enc = StateEncoder::new();
        let mut dec = StateDecoder::new();
        let snapshot = singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({ "revision": 0 }))],
        )]);
        assert_eq!(
            dec.decode_snapshot(&enc.encode_snapshot(&snapshot).unwrap())
                .unwrap(),
            snapshot
        );

        let first = state_update("state", 1, vec![set("revision", json!(1))]);
        let second = state_update("state", 2, vec![set("revision", json!(2))]);
        let first_wire = enc.encode_update(&first).unwrap();
        let second_wire = enc.encode_update(&second).unwrap();
        assert_eq!(
            wire_update_value(&first_wire)["ops"],
            json!([["s", ["revision"], 1]])
        );
        assert_eq!(
            wire_update_value(&second_wire)["ops"],
            json!([["#", 0, ["revision"]], ["s", 0, 2]])
        );
        assert_eq!(dec.decode_update(&first_wire).unwrap(), first);
        assert_eq!(dec.decode_update(&second_wire).unwrap(), second);
    }

    #[test]
    fn wire_values_round_trip_through_the_parsers() {
        let mut enc = StateEncoder::new();
        let snapshot = singleton(vec![
            MemberSnapshot::Method {
                name: "create".into(),
            },
            state_member("state", 4, vec![Op::Replace(json!({ "a": 1 }))]),
        ]);
        let wire = enc.encode_snapshot(&snapshot).unwrap();
        assert_eq!(
            parse_wire_snapshot(&wire_snapshot_value(&wire)).unwrap(),
            wire
        );
        for update in [
            state_update("state", 5, vec![set("a", json!(2))]),
            ProviderUpdate::Unavailable,
            ProviderUpdate::Closed {
                instance: Address {
                    key: "k".into(),
                    generation: 1,
                },
            },
        ] {
            let w = enc.encode_update(&update).unwrap();
            assert_eq!(parse_wire_update(&wire_update_value(&w)).unwrap(), w);
        }
    }

    #[test]
    fn resets_restart_path_dictionaries_at_the_new_baseline() {
        let mut enc = StateEncoder::new();
        let mut dec = StateDecoder::new();
        dec.decode_snapshot(
            &enc.encode_snapshot(&singleton(vec![state_member(
                "state",
                0,
                vec![Op::Replace(json!({ "before": 0 }))],
            )]))
            .unwrap(),
        )
        .unwrap();
        for sequence in 1..=2 {
            dec.decode_update(
                &enc.encode_update(&state_update(
                    "state",
                    sequence,
                    vec![set("before", json!(sequence))],
                ))
                .unwrap(),
            )
            .unwrap();
        }
        let reset = ProviderUpdate::Reset {
            snapshot: singleton(vec![state_member(
                "state",
                103,
                vec![Op::Replace(json!({ "after": 103 }))],
            )]),
        };
        assert_eq!(
            dec.decode_update(&enc.encode_update(&reset).unwrap())
                .unwrap(),
            reset
        );
        for sequence in 104..=106 {
            let update = state_update("state", sequence, vec![set("after", json!(sequence))]);
            assert_eq!(
                dec.decode_update(&enc.encode_update(&update).unwrap())
                    .unwrap(),
                update
            );
        }
    }

    #[test]
    fn dictionaries_are_isolated_between_states_and_subscriptions() {
        let snapshot = singleton(vec![
            state_member("left", 0, vec![Op::Replace(json!({ "revision": 0 }))]),
            state_member("right", 0, vec![Op::Replace(json!({ "revision": 0 }))]),
        ]);
        let (mut e1, mut d1, mut e2, mut d2) = (
            StateEncoder::new(),
            StateDecoder::new(),
            StateEncoder::new(),
            StateDecoder::new(),
        );
        d1.decode_snapshot(&e1.encode_snapshot(&snapshot).unwrap())
            .unwrap();
        d2.decode_snapshot(&e2.encode_snapshot(&snapshot).unwrap())
            .unwrap();
        let u = |m: &str, s: u64, r: u64| state_update(m, s, vec![set("revision", json!(r))]);
        let first_left = e1.encode_update(&u("left", 1, 1)).unwrap();
        let first_right = e1.encode_update(&u("right", 1, 1)).unwrap();
        let second_left = e1.encode_update(&u("left", 2, 2)).unwrap();
        let second_right = e1.encode_update(&u("right", 2, 2)).unwrap();
        // Each state interns on its own second use.
        assert_eq!(
            wire_update_value(&first_right)["ops"],
            json!([["s", ["revision"], 1]])
        );
        assert_eq!(
            wire_update_value(&second_right)["ops"],
            json!([["#", 0, ["revision"]], ["s", 0, 2]])
        );
        for (w, expected) in [
            (&first_left, u("left", 1, 1)),
            (&first_right, u("right", 1, 1)),
            (&second_left, u("left", 2, 2)),
            (&second_right, u("right", 2, 2)),
        ] {
            assert_eq!(d1.decode_update(w).unwrap(), expected);
        }
        // A second subscription's stream is independent.
        let independent = e2.encode_update(&u("left", 1, 1)).unwrap();
        assert_eq!(
            wire_update_value(&independent)["ops"],
            json!([["s", ["revision"], 1]])
        );
        assert_eq!(d2.decode_update(&independent).unwrap(), u("left", 1, 1));
        // A base update inside one state resets only that state's dictionary.
        let base = state_update("left", 3, vec![Op::Replace(json!({ "revision": 3 }))]);
        assert_eq!(
            d1.decode_update(&e1.encode_update(&base).unwrap()).unwrap(),
            base
        );
        let third_right = e1.encode_update(&u("right", 3, 3)).unwrap();
        assert_eq!(wire_update_value(&third_right)["ops"], json!([["s", 0, 3]]));
        assert_eq!(d1.decode_update(&third_right).unwrap(), u("right", 3, 3));
    }

    #[test]
    fn keyed_instance_codecs_follow_their_lifecycle() {
        let address = Address {
            key: "dialog-1".into(),
            generation: 1,
        };
        let instance = InstanceSnapshot {
            instance: Some(address.clone()),
            members: vec![state_member(
                "state",
                0,
                vec![Op::Replace(json!({ "n": 0 }))],
            )],
        };
        let mut enc = StateEncoder::new();
        let mut dec = StateDecoder::new();
        let empty = SubscriptionSnapshot {
            service_id: "pi.dialogs".into(),
            mode: Mode::Keyed,
            instances: vec![],
        };
        dec.decode_snapshot(&enc.encode_snapshot(&empty).unwrap())
            .unwrap();
        let spawned = ProviderUpdate::Spawned {
            instance: instance.clone(),
        };
        assert_eq!(
            dec.decode_update(&enc.encode_update(&spawned).unwrap())
                .unwrap(),
            spawned
        );
        // Spawning the same state twice is a protocol violation.
        assert!(enc.encode_update(&spawned).is_err());
        let update = ProviderUpdate::State {
            instance: Some(address.clone()),
            member: "state".into(),
            sequence: 1,
            ops: vec![set("n", json!(1))],
        };
        assert_eq!(
            dec.decode_update(&enc.encode_update(&update).unwrap())
                .unwrap(),
            update
        );
        let closed = ProviderUpdate::Closed { instance: address };
        dec.decode_update(&enc.encode_update(&closed).unwrap())
            .unwrap();
        // The closed instance's codec is gone.
        assert!(enc.encode_update(&update).is_err());
    }

    #[test]
    fn replica_hydrates_and_applies_contiguous_updates() {
        let mut replica = Replica::hydrate(singleton(vec![
            MemberSnapshot::Method {
                name: "select".into(),
            },
            state_member("state", 7, vec![Op::Replace(json!({ "revision": 0 }))]),
        ]))
        .unwrap();
        assert_eq!(replica.state("state"), Some(&json!({ "revision": 0 })));
        assert_eq!(replica.state("select"), None);
        let change = replica
            .apply(state_update("state", 8, vec![set("revision", json!(1))]))
            .unwrap();
        assert_eq!(
            change,
            Change::State {
                instance: None,
                member: "state".into()
            }
        );
        assert_eq!(replica.state("state"), Some(&json!({ "revision": 1 })));
        assert_eq!(replica.sequence(&None, "state"), Some(8));
    }

    #[test]
    fn a_sequence_gap_fails_the_replica() {
        let mut replica = Replica::hydrate(singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({}))],
        )]))
        .unwrap();
        let err = replica
            .apply(state_update("state", 2, vec![set("a", json!(1))]))
            .unwrap_err();
        assert_eq!(
            err,
            Error::SequenceGap {
                expected: 1,
                got: 2
            }
        );
        assert!(!replica.is_ready());
        assert_eq!(replica.state("state"), None);
        // Once failed, nothing more applies: the caller must resubscribe.
        assert!(replica.apply(state_update("state", 1, vec![])).is_err());
    }

    #[test]
    fn an_unapplicable_op_fails_the_replica() {
        let mut replica = Replica::hydrate(singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({ "n": 1 }))],
        )]))
        .unwrap();
        let bad = state_update(
            "state",
            1,
            vec![Op::Append(vec![delta::Seg::Key("n".into())], "x".into())],
        );
        assert!(replica.apply(bad).is_err());
        assert!(!replica.is_ready());
    }

    #[test]
    fn hydration_requires_a_base_batch() {
        let not_base = singleton(vec![state_member("state", 0, vec![set("a", json!(1))])]);
        assert!(Replica::hydrate(not_base).is_err());
        let empty = singleton(vec![state_member("state", 0, vec![])]);
        assert!(Replica::hydrate(empty).is_err());
        let dup = singleton(vec![
            MemberSnapshot::Method { name: "x".into() },
            MemberSnapshot::Method { name: "x".into() },
        ]);
        assert!(Replica::hydrate(dup).is_err());
    }

    #[test]
    fn unavailable_then_replaced_restores_the_singleton() {
        let mut replica = Replica::hydrate(singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({ "a": 1 }))],
        )]))
        .unwrap();
        assert_eq!(
            replica.apply(ProviderUpdate::Unavailable).unwrap(),
            Change::Unavailable
        );
        assert!(!replica.is_ready());
        assert_eq!(replica.state("state"), None);
        // No state update is valid while unavailable.
        assert!(replica.apply(state_update("state", 1, vec![])).is_err());

        let mut replica = Replica::hydrate(singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({ "a": 1 }))],
        )]))
        .unwrap();
        replica.apply(ProviderUpdate::Unavailable).unwrap();
        let replaced = ProviderUpdate::Replaced {
            snapshot: InstanceSnapshot {
                instance: None,
                members: vec![state_member(
                    "state",
                    40,
                    vec![Op::Replace(json!({ "a": 2 }))],
                )],
            },
        };
        replica.apply(replaced).unwrap();
        assert!(replica.is_ready());
        assert_eq!(replica.state("state"), Some(&json!({ "a": 2 })));
        replica
            .apply(state_update("state", 41, vec![set("a", json!(3))]))
            .unwrap();
    }

    #[test]
    fn a_reset_installs_a_new_baseline_and_checks_the_service() {
        let mut replica = Replica::hydrate(singleton(vec![state_member(
            "state",
            0,
            vec![Op::Replace(json!({ "a": 1 }))],
        )]))
        .unwrap();
        let reset = ProviderUpdate::Reset {
            snapshot: singleton(vec![state_member(
                "state",
                103,
                vec![Op::Replace(json!({ "a": 9 }))],
            )]),
        };
        replica.apply(reset).unwrap();
        assert_eq!(replica.state("state"), Some(&json!({ "a": 9 })));
        replica
            .apply(state_update("state", 104, vec![set("a", json!(10))]))
            .unwrap();

        let mut other = Replica::hydrate(singleton(vec![])).unwrap();
        let wrong = ProviderUpdate::Reset {
            snapshot: SubscriptionSnapshot {
                service_id: "pi.other".into(),
                mode: Mode::Singleton,
                instances: vec![],
            },
        };
        assert!(other.apply(wrong).is_err());
    }

    #[test]
    fn keyed_updates_for_stale_generations_are_ignored() {
        let one = Address {
            key: "d".into(),
            generation: 1,
        };
        let two = Address {
            key: "d".into(),
            generation: 2,
        };
        let snapshot = SubscriptionSnapshot {
            service_id: "pi.dialogs".into(),
            mode: Mode::Keyed,
            instances: vec![InstanceSnapshot {
                instance: Some(two.clone()),
                members: vec![state_member(
                    "state",
                    0,
                    vec![Op::Replace(json!({ "n": 0 }))],
                )],
            }],
        };
        let mut replica = Replica::hydrate(snapshot).unwrap();
        let stale = ProviderUpdate::State {
            instance: Some(one),
            member: "state".into(),
            sequence: 1,
            ops: vec![set("n", json!(1))],
        };
        assert_eq!(replica.apply(stale).unwrap(), Change::Ignored);
        assert!(replica.is_ready());
        let live = ProviderUpdate::State {
            instance: Some(two.clone()),
            member: "state".into(),
            sequence: 1,
            ops: vec![set("n", json!(1))],
        };
        replica.apply(live).unwrap();
        assert_eq!(
            replica.instance_state(&Some(two), "state"),
            Some(&json!({ "n": 1 }))
        );
    }
}
