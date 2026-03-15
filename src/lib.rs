mod error;
use corepc_node::P2P;
use std::{
    fs::File,
    io::{self, BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering::Relaxed},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};
use temp_dir::TempDir;

pub use error::Error;

/// Storage strategy for the blindbit-oracle server.
///
/// Each variant enables a specific data storage model. The two endpoints
/// `/tweak-index` and `/tweaks` serve different data structures:
///
/// - `/tweak-index` → queries block-level index (TweakIndex or TweakIndexDust)
/// - `/tweaks` → queries per-transaction storage (individual Tweaks)
///
/// **Important:** These are mutually exclusive storage strategies. Only one
/// endpoint will return data depending on which storage is enabled.
///
/// See also: <https://github.com/setavenger/blindbit-oracle>
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Storage {
    /// Block-level index without dust filtering.
    ///
    /// - Server config: `tweaks_full_basic=1`
    /// - Storage: TweakIndex (33-byte tweaks per block)
    /// - `/tweak-index`: works (dustLimit=0 only, errors on dustLimit>0)
    /// - `/tweaks`: returns empty `[]`
    /// - Use case: Simple full-block scanning, no dust optimization
    FullBasic,

    /// Block-level index WITH dust filtering support.
    ///
    /// - Server config: `tweaks_full_with_dust_filter=1`
    /// - Storage: TweakIndexDust (tweaks + highest output value per block)
    /// - `/tweak-index`: works with optional dustLimit filtering
    /// - `/tweaks`: returns empty `[]`
    /// - Use case: Full-block scanning with client-side dust filtering
    DustFilter,

    /// Per-transaction storage with dust filtering and cut-through.
    ///
    /// - Server config: `tweaks_cut_through_with_dust_filter=1`
    /// - Storage: Individual Tweaks per transaction (prunable)
    /// - `/tweak-index`: returns empty `[]`
    /// - `/tweaks`: works with optional dustLimit filtering
    /// - Use case: Space-efficient storage, tweaks pruned when outputs spent
    ///
    /// **Note:** Cannot be combined with `tweaks_only=true` (requires UTXO tracking).
    DustFilterCutThrough,
}

impl From<&Storage> for (u8, u8, u8) {
    fn from(value: &Storage) -> Self {
        match value {
            Storage::FullBasic => (1, 0, 0),
            Storage::DustFilter => (0, 1, 0),
            Storage::DustFilterCutThrough => (0, 0, 1),
        }
    }
}

impl Storage {
    /// Returns the server config flags as (tweaks_full_basic, tweaks_full_with_dust_filter, tweaks_cut_through_with_dust_filter)
    pub fn values(&self) -> (u8, u8, u8) {
        self.into()
    }
}

#[derive(Debug, PartialEq, Eq, Clone)]
#[non_exhaustive]
pub struct Conf<'a> {
    /// command line arguments
    pub args: Vec<&'a str>,

    /// Try to spawn the process `attempt` time
    ///
    /// The OS is giving available ports to use, however, they aren't booked, so it could rarely
    /// happen they are used at the time the process is spawn. When retrying other available ports
    /// are returned reducing the probability of conflicts to negligible.
    attempts: u8,

    /// The ip to bind to
    pub ip: Option<String>,

    /// The port to listen on
    pub port: Option<u16>,

    // Path to the binary
    pub binary: Option<String>,

    /// Storage strategy for tweaks
    pub storage: Storage,

    /// Skip UTXO processing (filters, spent index, etc.)
    ///
    /// When `true`, the server only handles tweak storage, not UTXO tracking.
    /// This saves storage and processing but disables some features.
    ///
    /// **Note:** Cannot be `true` when using `Storage::DustFilterCutThrough`
    /// (cut-through requires UTXO tracking to prune spent outputs).
    pub tweaks_only: bool,
}

impl Default for Conf<'_> {
    fn default() -> Self {
        Self {
            args: Vec::new(),
            attempts: 5,
            ip: None,
            port: None,
            binary: None,
            storage: Storage::DustFilterCutThrough,
            tweaks_only: false,
        }
    }
}

impl Conf<'_> {
    /// Create a new Conf with the specified storage strategy
    pub fn with_storage(storage: Storage) -> Self {
        Self {
            storage,
            ..Default::default()
        }
    }

    /// Create a new Conf with tweaks_only mode and given storage
    ///
    /// # Panics
    /// Panics if storage is `DustFilterCutThrough` (incompatible with tweaks_only)
    pub fn tweaks_only_with(storage: Storage) -> Self {
        assert!(
            storage != Storage::DustFilterCutThrough,
            "tweaks_only cannot be used with DustFilterCutThrough (requires UTXO tracking)"
        );
        Self {
            storage,
            tweaks_only: true,
            ..Default::default()
        }
    }
}

/// Returns a non-used local port if available.
///
/// Note there is a race condition during the time the method check availability and the caller
pub fn get_available_port() -> Result<u16, Error> {
    // using 0 as port let the system assign a port available
    let t = TcpListener::bind(("127.0.0.1", 0))?; // 0 means the OS choose a free port
    Ok(t.local_addr().map(|s| s.port())?)
}

/// Struct representing the electrs process with related information
pub struct BlindbitD {
    /// Process child handle, used to terminate the process when this struct is dropped
    pub process: Child,
    /// Work directory, removed when dropped
    pub work_dir: TempDir,
    /// A buffer receiving stdout and stderr
    pub logs: Receiver<String>,
    /// The port we listen to
    pub port: u16,
    /// the address we listen to
    pub addr: String,
    /// Path to the binary
    pub binary: PathBuf,
    /// Bitcoind
    pub bitcoin: Option<corepc_node::Node>,
    /// Electrsd
    #[cfg(feature = "electrum")]
    pub electrsd: Option<electrsd::ElectrsD>,
}

fn try_read_line<R: BufRead>(reader: &mut R) -> io::Result<Option<String>> {
    let mut buffer = Vec::new();
    // Try to read until a newline
    match reader.read_until(b'\n', &mut buffer)? {
        0 => Ok(None), // EOF reached or no data available
        _ => {
            if let Ok(line) = String::from_utf8(buffer) {
                Ok(Some(line))
            } else {
                Ok(None)
            }
        }
    }
}

impl BlindbitD {
    /// Create a new blindbit process
    pub fn new() -> Result<BlindbitD, Error> {
        BlindbitD::with_conf(&Conf::default())
    }

    /// Create a new process using given [Conf]
    pub fn with_conf(conf: &Conf) -> Result<BlindbitD, Error> {
        let mut args = conf.args.clone();
        let ip = conf.ip.clone().unwrap_or("127.0.0.1".into());
        let port = conf.port.unwrap_or(get_available_port()?);

        // Use CARGO_MANIFEST_DIR for reliable path resolution in workspace builds
        let mut bin_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        bin_dir.push("bin");
        bin_dir.push("blindbit_bcd562f");

        let bin = if let Some(bin) = conf.binary.clone() {
            bin
        } else if let Some(bin) = &bin_dir.to_str() {
            bin.to_string()
        } else {
            panic!("no valid binary path")
        };

        let exe = Path::new(&bin);
        if !exe.exists() {
            panic!("path {:?} does not exists!", exe);
        }
        if !exe.is_file() {
            panic!(" path {:?} is not a file!", exe);
        }

        // create the temp dir
        let work_dir = TempDir::with_prefix("blindbit_").unwrap();

        // launch bitcoind
        let mut bitcoin_conf = corepc_node::Conf::default();
        bitcoin_conf.args.push("-txindex");
        bitcoin_conf.p2p = P2P::Yes;
        let bitcoind = corepc_node::Node::from_downloaded_with_conf(&bitcoin_conf).unwrap();
        let bitcoind_addr = bitcoind.params.rpc_socket;
        let bitcoind_cookie = bitcoind.params.cookie_file.clone().canonicalize().unwrap();

        #[cfg(feature = "electrum")]
        let electrsd = {
            let electrs_exe = electrsd::downloaded_exe_path()
                .expect("electrs binary not found");
            electrsd::ElectrsD::new(electrs_exe, &bitcoind)
                .expect("failed to spawn electrsd")
        };

        // config file
        let config_path = work_dir.child("blindbit.toml");
        let mut file = File::create(config_path.clone())?;
        writeln!(&file, "host = \"{ip}:{port}\"").unwrap();
        writeln!(file, "chain = \"regtest\"").unwrap();
        writeln!(file, "rpc_endpoint = \"http://{bitcoind_addr}\"").unwrap();
        writeln!(
            file,
            "cookie_path = \"{}\"",
            bitcoind_cookie.to_str().unwrap()
        )
        .unwrap();
        writeln!(file, "sync_start_height = 1").unwrap();
        writeln!(file, "max_parallel_tweak_computations = 4").unwrap();
        writeln!(file, "max_parallel_requests = 4").unwrap();
        let tweaks_only = u8::from(conf.tweaks_only);
        let (a, b, c) = conf.storage.values();
        writeln!(file, "tweaks_only = {tweaks_only}").unwrap();
        writeln!(file, "tweaks_full_basic = {a}").unwrap();
        writeln!(file, "tweaks_full_with_dust_filter = {b}").unwrap();
        writeln!(file, "tweaks_cut_through_with_dust_filter = {c}").unwrap();
        drop(file);

        let mut file = File::open(config_path).unwrap();
        let mut content = String::new();
        file.read_to_string(&mut content).unwrap();

        // config
        args.push("--datadir");
        let cfg_path = work_dir.path();
        let path = cfg_path.to_str().expect("hardcoded");
        args.push(path);

        let (sender, logs) = mpsc::channel();

        let mut p = None;
        #[allow(clippy::never_loop)]
        'f: for _ in 0..bitcoin_conf.attempts {
            let mut process = Command::new(exe)
                .args(args.clone())
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;
            let timeout = Instant::now() + Duration::from_secs(3);
            let stdout = process.stdout.take().unwrap();
            let mut stdout_reader = BufReader::new(stdout);
            let s = sender.clone();
            let stop = Arc::new(AtomicBool::new(false));
            let stop2 = stop.clone();
            thread::spawn(move || loop {
                if let Ok(Some(line)) = try_read_line(&mut stdout_reader) {
                    let _ = s.send(line);
                } else if stop2.load(Relaxed) {
                    break;
                }
            });

            loop {
                if Instant::now() > timeout {
                    let _ = process.kill();
                    stop.store(true, Relaxed);
                    return Err(Error::Start);
                } else if let Ok(log) = logs.try_recv() {
                    if log.contains("Listening and serving HTTP") {
                        p = Some(process);
                        break 'f;
                    } else {
                        sleep(Duration::from_millis(10));
                    }
                }
            }
        }
        let mut process = if let Some(p) = p {
            p
        } else {
            panic!(
                "Fail to start BlindbitD after {} attempts",
                bitcoin_conf.attempts
            );
        };
        let stderr = process.stderr.take().unwrap();

        thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                sender.send(line.unwrap()).unwrap();
            }
        });

        Ok(BlindbitD {
            process,
            work_dir,
            logs,
            addr: ip.clone(),
            port,
            binary: exe.to_path_buf(),
            bitcoin: Some(bitcoind),
            #[cfg(feature = "electrum")]
            electrsd: Some(electrsd),
        })
    }

    /// Return the current workdir path of the running electrs
    pub fn workdir(&self) -> PathBuf {
        self.work_dir.path().to_path_buf()
    }

    /// terminate the process
    pub fn kill(&mut self) -> Result<(), Error> {
        self.inner_kill()?;
        // Wait for the process to exit
        match self.process.wait() {
            Ok(_) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// clear the log buffer
    pub fn clear_logs(&mut self) {
        while self.logs.try_recv().is_ok() {}
    }

    fn inner_kill(&mut self) -> Result<(), Error> {
        // Send SIGINT signal to electrsd
        Ok(nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(self.process.id() as i32),
            nix::sys::signal::SIGINT,
        )?)
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}", self.addr, self.port)
    }

    pub fn bitcoin(&mut self) -> Option<corepc_node::Node> {
        self.bitcoin.take()
    }

    #[cfg(feature = "electrum")]
    pub fn electrum(&mut self) -> Option<electrsd::ElectrsD> {
        self.electrsd.take()
    }
}

impl Drop for BlindbitD {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
