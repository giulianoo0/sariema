// SPDX-License-Identifier: Apache-2.0

//! The window's host for the game's messages: its saves kept in one JSON
//! file per game in a directory, in the format `run --headless --saves`
//! reads, its `save_load` answered from that file, and `host_exit` closing
//! the window. The other messages are taken and let go.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use sariema_engine::Engine;
use sariema_runtime::outbox::HostOut;
use sariema_runtime::sim::Logic;

use crate::output::Failure;
use crate::simulate::read_saves;

/// The folder beside the pack the saves go in without `--save-dir`.
pub const DEFAULT_DIR: &str = "saves";

pub struct SaveDir {
    file: PathBuf,
    saves: BTreeMap<String, Vec<u8>>,
    taken: Vec<(u64, HostOut)>,
}

impl SaveDir {
    /// The saves of the game in `pack`, kept in `dir` as `<pack name>.json`;
    /// without `dir`, in [`DEFAULT_DIR`] beside the pack.
    pub fn open(pack: &Path, dir: Option<&Path>) -> Result<SaveDir, Failure> {
        let dir = match dir {
            Some(dir) => dir.to_path_buf(),
            None => pack.parent().unwrap_or(Path::new(".")).join(DEFAULT_DIR),
        };
        let mut name = pack.file_stem().unwrap_or(pack.as_os_str()).to_os_string();
        name.push(".json");
        let file = dir.join(name);
        let saves = if file.exists() {
            read_saves(&file)?
        } else {
            BTreeMap::new()
        };
        Ok(SaveDir {
            file,
            saves,
            taken: Vec::new(),
        })
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Takes the game's messages after a frame's steps: keeps its saves,
    /// writing the file once when they changed, and answers its loads.
    /// Whether the game asked the host to exit.
    pub fn serve<L: Logic>(&mut self, engine: &mut Engine<L>) -> Result<bool, Failure> {
        let (mut changed, mut exit) = (false, false);
        self.taken.extend(engine.take_host());
        for (_, message) in self.taken.drain(..) {
            match message {
                HostOut::Save { key, value } => {
                    self.saves.insert(key, value);
                    changed = true;
                }
                HostOut::LoadSaves => {
                    let all = self.saves.iter().map(|(k, v)| (k.clone(), v.clone()));
                    engine.answer_saves(all.collect());
                }
                HostOut::Exit => exit = true,
                _ => {}
            }
        }
        if changed {
            self.write()?;
        }
        Ok(exit)
    }

    /// Writes the saves beside the file and moves them over it, so a crash
    /// mid-write leaves the last whole file.
    fn write(&self) -> Result<(), Failure> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(|e| Failure::io(dir, e))?;
        }
        let json = serde_json::to_vec(&self.saves).expect("byte arrays by key are JSON");
        let mut next = self.file.clone().into_os_string();
        next.push(".part");
        let next = PathBuf::from(next);
        fs::write(&next, json).map_err(|e| Failure::io(&next, e))?;
        fs::rename(&next, &self.file).map_err(|e| Failure::io(&self.file, e))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use sariema_engine::{Assets, EngineSettings};
    use sariema_format::script::{Command, Event, records};
    use sariema_runtime::input::Actions;
    use sariema_runtime::settings::Settings;
    use sariema_runtime::sim::{Simulation, Stop};

    use super::*;

    /// Asks for its saves at its first tick; with none, saves `best` and
    /// asks to exit. Logs what it loads.
    struct Saver {
        tick: u64,
        loaded: Rc<RefCell<Vec<String>>>,
    }

    impl Logic for Saver {
        fn tick(&mut self, events: &[u8], _: &[u32], out: &mut Vec<u8>) -> Result<(), Stop> {
            out.clear();
            for event in records::<Event>(events).flatten() {
                match event {
                    Event::SaveLoaded { key, value } => {
                        self.loaded.borrow_mut().push(format!("{key}={value:?}"))
                    }
                    Event::SaveLoadDone {} if self.loaded.borrow().is_empty() => {
                        Command::Save {
                            key: "best",
                            value: &[12],
                        }
                        .write(out);
                        Command::HostExit {}.write(out);
                    }
                    _ => {}
                }
            }
            if self.tick == 0 {
                Command::SaveLoad {}.write(out);
            }
            self.tick += 1;
            Ok(())
        }
    }

    fn engine(loaded: &Rc<RefCell<Vec<String>>>) -> Engine<Saver> {
        let logic = Saver {
            tick: 0,
            loaded: loaded.clone(),
        };
        let sim = Simulation::new(Settings::default(), Vec::new(), logic, 1);
        let assets = Assets {
            room: None,
            collision: None,
            actors: Vec::new(),
            sounds: None,
            hud: None,
        };
        Engine::new(sim, assets, EngineSettings::default()).unwrap()
    }

    /// Steps the engine, serving its messages each step, until the game
    /// asks to exit or `steps` run; whether it asked.
    fn play(engine: &mut Engine<Saver>, saves: &mut SaveDir, steps: u32) -> bool {
        engine.answers_saves(true);
        let actions = Actions::for_settings(&engine.state().settings.input);
        for _ in 0..steps {
            engine.step(&actions);
            if saves.serve(engine).unwrap() {
                return true;
            }
        }
        false
    }

    #[test]
    fn saves_go_to_the_directory_and_come_back_in_the_next_run() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pack = tmp.path().join("games/climb.srmg");
        let loaded = Rc::new(RefCell::new(Vec::new()));
        let mut saves = SaveDir::open(&pack, None).unwrap();
        assert_eq!(saves.file(), tmp.path().join("games/saves/climb.json"));
        assert!(
            play(&mut engine(&loaded), &mut saves, 10),
            "the game asked to exit"
        );
        assert!(loaded.borrow().is_empty());
        let written = fs::read_to_string(saves.file()).unwrap();
        assert_eq!(written, r#"{"best":[12]}"#);

        let mut again = SaveDir::open(&pack, None).unwrap();
        assert!(!play(&mut engine(&loaded), &mut again, 10));
        assert_eq!(*loaded.borrow(), ["best=[12]"]);
    }

    #[test]
    fn a_save_dir_given_is_used() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir = tmp.path().join("elsewhere");
        let saves = SaveDir::open(&tmp.path().join("climb.srmg"), Some(&dir)).unwrap();
        assert_eq!(saves.file(), dir.join("climb.json"));
    }

    #[test]
    fn packs_that_differ_after_a_dot_keep_their_own_saves() {
        let tmp = tempfile::TempDir::new().unwrap();
        let v1 = SaveDir::open(&tmp.path().join("climb.v1.srmg"), None).unwrap();
        let v2 = SaveDir::open(&tmp.path().join("climb.v2.srmg"), None).unwrap();
        assert_eq!(v1.file(), tmp.path().join("saves/climb.v1.json"));
        assert_eq!(v2.file(), tmp.path().join("saves/climb.v2.json"));
    }
}
