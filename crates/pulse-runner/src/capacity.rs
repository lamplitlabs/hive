//! One capacity model for the fleet: every machine works out how many runs
//! it takes from its own hardware, the fleet's total is the sum over the
//! machines connected, and a project's share of it follows from how many
//! projects have work. No owner number is needed; the one optional override
//! is `capacity` (this machine's slots), and `pulse stop` pauses.
//!
//! Per machine: `max(1, min(physical cores, RAM GB / 3))` - a run is a
//! worker clone with its install, its test run and the harness, about 3 GB
//! and one core at the peak. The load governor ([`crate::daemon_load`])
//! still backs off live on load and free memory with internal constants.
//!
//! Per project: `ceil(fleet / projects with work)`, never under one, so a
//! lone busy project can use every slot and several share them fairly.

/// Megabytes of RAM one run is taken to need.
pub const MB_PER_RUN: i64 = 3072;

/// Runs a machine with `cores` physical cores and `mem_mb` of RAM takes at
/// once: the smaller of its cores and one run per [`MB_PER_RUN`], never
/// under one. An unknown memory size (0) leaves the cores alone to decide.
pub fn machine_slots(cores: i32, mem_mb: i64) -> i32 {
    let cores = cores.max(1);
    let by_mem = if mem_mb > 0 {
        (mem_mb / MB_PER_RUN).clamp(1, i64::from(i32::MAX)) as i32
    } else {
        cores
    };
    cores.min(by_mem).max(1)
}

/// A project's share of the fleet: `ceil(total / busy)`, at least one;
/// `busy` counts the projects with work (0 counts as one).
pub fn per_project(total: i32, busy: i32) -> i32 {
    let total = total.max(1);
    let busy = busy.max(1);
    ((total + busy - 1) / busy).max(1)
}

/// The `capacity` override: None for auto, Some(n) for a fixed number.
pub fn override_slots(s: &pulse_store::Store) -> Option<i32> {
    let v = s.setting("capacity", "auto");
    let v = v.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("auto") || v.eq_ignore_ascii_case("off") {
        return None;
    }
    v.parse::<i64>()
        .ok()
        .map(|n| n.clamp(0, i64::from(i32::MAX)) as i32)
}

/// This machine's slots: the `capacity` override, else its hardware.
pub fn local_slots(s: &pulse_store::Store) -> i32 {
    override_slots(s).unwrap_or_else(|| {
        let m = crate::daemon_load::Machine::detect();
        machine_slots(m.cores, m.mem_mb)
    })
}

/// One machine's share of the fleet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineSlots {
    pub name: String,
    pub slots: i32,
}

/// The fleet as it stands: each active machine and its slots, the total,
/// the projects with work and a project's share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fleet {
    pub machines: Vec<MachineSlots>,
    pub total: i32,
    pub busy_projects: i32,
    pub per_project: i32,
}

impl Fleet {
    /// "slots 18 = this machine 10 + vm-1 8, per project 6".
    pub fn line(&self) -> String {
        let parts = self
            .machines
            .iter()
            .map(|m| {
                let name = if m.name == crate::executor::LOCAL_EXECUTOR {
                    "this machine"
                } else {
                    m.name.as_str()
                };
                format!("{name} {}", m.slots)
            })
            .collect::<Vec<_>>()
            .join(" + ");
        format!(
            "slots {} = {parts}, per project {}",
            self.total, self.per_project
        )
    }
}

/// Reads the fleet from the store: this machine, then every active remote
/// executor with its `workers` (sized from its hardware by its probe).
pub fn fleet(s: &pulse_store::Store) -> Fleet {
    let mut machines = vec![MachineSlots {
        name: crate::executor::LOCAL_EXECUTOR.to_string(),
        slots: local_slots(s),
    }];
    if let Ok(conn) = s.conn() {
        if let Ok(mut stmt) = conn.prepare(
            "SELECT name, workers FROM executors WHERE status = 'active' ORDER BY name",
        ) && let Ok(rows) = stmt.query_map([], |r| {
            Ok(MachineSlots {
                name: r.get(0)?,
                slots: r.get::<_, i64>(1)?.clamp(0, i64::from(i32::MAX)) as i32,
            })
        }) {
            machines.extend(rows.flatten());
        }
    }
    let busy: i32 = s
        .conn()
        .ok()
        .and_then(|conn| {
            conn.query_row(
                "SELECT COUNT(DISTINCT project) FROM jobs WHERE kind != 'experiment' AND status IN ('queued', 'leased')",
                [],
                |r| r.get::<_, i64>(0),
            )
            .ok()
        })
        .unwrap_or(0) as i32;
    let total: i32 = machines.iter().map(|m| m.slots).sum();
    Fleet {
        per_project: per_project(total, busy),
        machines,
        total,
        busy_projects: busy,
    }
}

/// A project's share of the fleet now.
pub fn parallel_per_project(s: &pulse_store::Store) -> i32 {
    #[cfg(test)]
    if let Ok((n, true)) = s.meta(PIN_KEY)
        && let Ok(n) = n.parse::<i32>()
    {
        return n.max(1);
    }
    fleet(s).per_project
}

#[cfg(test)]
const PIN_KEY: &str = "test.per_project";

/// Tests fix a project's share on their own store, whatever the fleet says.
#[cfg(test)]
pub(crate) fn pin_per_project(s: &pulse_store::Store, n: i32) {
    s.set_meta(PIN_KEY, &n.to_string()).expect("pin per project");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_takes_the_smaller_of_its_cores_and_its_memory() {
        for (name, cores, mem_mb, want) in [
            ("16 GB, 12 cores: five", 12, 16384, 5),
            ("64 GB, 32 cores: twenty-one, no cap of four", 32, 65536, 21),
            ("128 GB, 16 cores: the cores", 16, 131072, 16),
            ("8 GB, 8 cores: two", 8, 8192, 2),
            ("2 GB: never under one", 2, 2048, 1),
            ("memory unknown: the cores", 6, 0, 6),
            ("no cores reported: one", 0, 0, 1),
        ] {
            assert_eq!(machine_slots(cores, mem_mb), want, "{name}");
        }
    }

    #[test]
    fn a_project_gets_a_fair_share_and_a_lone_project_everything() {
        assert_eq!(per_project(18, 1), 18);
        assert_eq!(per_project(18, 0), 18, "no work yet counts as one");
        assert_eq!(per_project(18, 3), 6);
        assert_eq!(per_project(18, 4), 5, "rounded up");
        assert_eq!(per_project(2, 5), 1, "never under one");
        assert_eq!(per_project(0, 1), 1);
    }

    #[test]
    fn the_fleet_sums_its_machines_and_says_so() {
        let f = Fleet {
            machines: vec![
                MachineSlots {
                    name: crate::executor::LOCAL_EXECUTOR.into(),
                    slots: 10,
                },
                MachineSlots {
                    name: "vm-1".into(),
                    slots: 8,
                },
            ],
            total: 18,
            busy_projects: 3,
            per_project: per_project(18, 3),
        };
        assert_eq!(f.line(), "slots 18 = this machine 10 + vm-1 8, per project 6");
    }
}
