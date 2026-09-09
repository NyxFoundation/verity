use std::collections::BTreeMap;
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use libssz::SszDecode;
use libssz_merkle::HashTreeRoot;
use serde::Deserialize;
use verity_chain::{LeanSszType, lean_hash_tree_root, native_hash_tree_root};
use verity_types::{
    AggregatedAttestation, AggregatedAttestations, AggregationBits, AttestationData, Block,
    BlockBody, BlockHeader, Checkpoint, GenesisConfig, HistoricalBlockHashes, JustificationRoots,
    JustificationValidators, JustifiedSlots, Slot, State, Validator, ValidatorIndex, Validators,
};

const SAMPLE_COUNT: usize = 15;
const SAMPLE_FLOOR: Duration = Duration::from_millis(25);

#[derive(Debug)]
struct Measurement {
    median_ns: f64,
    low_ns: f64,
    high_ns: f64,
}

impl Measurement {
    fn roots_per_second(&self) -> f64 {
        1_000_000_000.0 / self.median_ns
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureCase {
    type_name: String,
    serialized: String,
    #[serde(default)]
    rejection_reason: Option<String>,
}

fn main() {
    let cases = synthetic_cases();
    println!(
        "| input | SSZ bytes | libssz median | PR #132 median | slowdown | libssz roots/s | PR #132 roots/s |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    benchmark("synthetic/BlockHeader", &cases.0);
    benchmark("synthetic/AttestationData", &cases.1);
    benchmark("synthetic/BlockBody-64", &cases.2);
    benchmark("synthetic/Block-64", &cases.3);
    benchmark("synthetic/State-256v-1024h", &cases.4);
    benchmark_fixtures();
}

fn benchmark<T>(name: &str, value: &T)
where
    T: HashTreeRoot + LeanSszType,
{
    let expected = native_hash_tree_root(value);
    let actual = lean_hash_tree_root(value)
        .unwrap_or_else(|error| panic!("{name}: experimental Lean backend is required: {error}"));
    assert_eq!(actual, expected, "{name}: root parity");

    let native = measure(|| black_box(native_hash_tree_root(black_box(value))));
    let lean = measure(|| {
        black_box(
            lean_hash_tree_root(black_box(value)).expect("parity check established valid input"),
        )
    });
    print_row(name, value.to_ssz().len(), &native, &lean);
}

fn measure<T>(mut operation: impl FnMut() -> T) -> Measurement {
    for _ in 0..3 {
        black_box(operation());
    }
    let iterations = calibrated_iterations(&mut operation);
    let mut samples = Vec::with_capacity(SAMPLE_COUNT);
    for _ in 0..SAMPLE_COUNT {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        samples.push(start.elapsed().as_nanos() as f64 / iterations as f64);
    }
    samples.sort_by(f64::total_cmp);
    Measurement {
        median_ns: samples[SAMPLE_COUNT / 2],
        low_ns: samples[1],
        high_ns: samples[SAMPLE_COUNT - 2],
    }
}

fn calibrated_iterations<T>(operation: &mut impl FnMut() -> T) -> u64 {
    let mut iterations = 1u64;
    loop {
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(operation());
        }
        if start.elapsed() >= SAMPLE_FLOOR || iterations >= 1 << 24 {
            return iterations;
        }
        iterations *= 2;
    }
}

fn print_row(name: &str, bytes: usize, native: &Measurement, lean: &Measurement) {
    println!(
        "| {name} | {bytes} | {} | {} | {:.1}× | {:.1} | {:.1} |",
        duration(native),
        duration(lean),
        lean.median_ns / native.median_ns,
        native.roots_per_second(),
        lean.roots_per_second()
    );
}

fn duration(measurement: &Measurement) -> String {
    format!(
        "{} ({}–{})",
        unit(measurement.median_ns),
        unit(measurement.low_ns),
        unit(measurement.high_ns)
    )
}

fn unit(nanoseconds: f64) -> String {
    if nanoseconds >= 1_000_000.0 {
        format!("{:.3} ms", nanoseconds / 1_000_000.0)
    } else if nanoseconds >= 1_000.0 {
        format!("{:.3} µs", nanoseconds / 1_000.0)
    } else {
        format!("{nanoseconds:.1} ns")
    }
}

fn synthetic_cases() -> (BlockHeader, AttestationData, BlockBody, Block, State) {
    let header = header(7);
    let data = attestation_data(11);
    let attestations = (0..64)
        .map(|index| AggregatedAttestation {
            aggregation_bits: bits(256, index),
            data: attestation_data(index as u64),
        })
        .collect::<Vec<_>>();
    let body = BlockBody {
        attestations: AggregatedAttestations::try_from(attestations)
            .expect("64 attestations fit the consensus bound"),
    };
    let block = Block {
        slot: header.slot,
        proposer_index: header.proposer_index,
        parent_root: header.parent_root,
        state_root: header.state_root,
        body: body.clone(),
    };
    (header, data, body, block, synthetic_state())
}

fn synthetic_state() -> State {
    let validators = (0..256)
        .map(|index| Validator {
            attestation_public_key: bytes::<52>(index),
            proposal_public_key: bytes::<52>(index + 1),
            index: ValidatorIndex(index),
        })
        .collect::<Vec<_>>();
    let history = (0..1024).map(bytes::<32>).collect::<Vec<_>>();
    State {
        config: GenesisConfig { genesis_time: 1 },
        slot: Slot(1024),
        latest_block_header: header(1024),
        latest_justified: checkpoint(1020),
        latest_finalized: checkpoint(1016),
        historical_block_hashes: HistoricalBlockHashes::try_from(history.clone()).unwrap(),
        justified_slots: JustifiedSlots::try_from(bit_values(1024, 3)).unwrap(),
        validators: Validators::try_from(validators).unwrap(),
        justifications_roots: JustificationRoots::try_from(history).unwrap(),
        justifications_validators: JustificationValidators::try_from(bit_values(768, 5)).unwrap(),
    }
}

fn header(seed: u64) -> BlockHeader {
    BlockHeader {
        slot: Slot(seed),
        proposer_index: ValidatorIndex(seed % 256),
        parent_root: bytes(seed),
        state_root: bytes(seed + 1),
        body_root: bytes(seed + 2),
    }
}

fn attestation_data(seed: u64) -> AttestationData {
    AttestationData {
        slot: Slot(seed),
        head: checkpoint(seed),
        target: checkpoint(seed.saturating_sub(1)),
        source: checkpoint(seed.saturating_sub(2)),
    }
}

fn checkpoint(seed: u64) -> Checkpoint {
    Checkpoint {
        root: bytes(seed),
        slot: Slot(seed),
    }
}

fn bytes<const N: usize>(seed: u64) -> [u8; N] {
    std::array::from_fn(|index| {
        seed.wrapping_mul(31)
            .wrapping_add(index as u64 * 17)
            .to_le_bytes()[0]
    })
}

fn bits(length: usize, seed: usize) -> AggregationBits {
    AggregationBits::try_from(bit_values(length, seed + 2)).unwrap()
}

fn bit_values(length: usize, stride: usize) -> Vec<bool> {
    (0..length).map(|index| index % stride == 0).collect()
}

fn benchmark_fixtures() {
    let Some(root) = std::env::var_os("VERITY_FIXTURES").map(PathBuf::from) else {
        eprintln!("VERITY_FIXTURES is unset; fixture cases skipped");
        return;
    };
    let mut paths = Vec::new();
    collect_json(&root, &mut paths);
    paths.sort();
    let mut found = BTreeMap::new();
    for path in paths {
        load_fixture_file(&path, &mut found);
    }
    dispatch_fixtures(found);
}

fn collect_json(path: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(&path, output);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "json")
            && path
                .components()
                .any(|part| part.as_os_str() == "test_consensus_containers")
        {
            output.push(path);
        }
    }
}

fn load_fixture_file(path: &Path, found: &mut BTreeMap<String, Vec<u8>>) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let Ok(cases) = serde_json::from_str::<BTreeMap<String, FixtureCase>>(&text) else {
        return;
    };
    for case in cases.into_values() {
        if case.rejection_reason.is_none()
            && supported_fixture(&case.type_name)
            && let Ok(bytes) = from_hex(&case.serialized)
        {
            found.entry(case.type_name).or_insert(bytes);
        }
    }
}

fn supported_fixture(name: &str) -> bool {
    matches!(
        name,
        "State" | "Block" | "BlockBody" | "BlockHeader" | "AttestationData"
    )
}

fn dispatch_fixtures(fixtures: BTreeMap<String, Vec<u8>>) {
    for (name, bytes) in fixtures {
        match name.as_str() {
            "State" => decode_and_benchmark::<State>(&name, &bytes),
            "Block" => decode_and_benchmark::<Block>(&name, &bytes),
            "BlockBody" => decode_and_benchmark::<BlockBody>(&name, &bytes),
            "BlockHeader" => decode_and_benchmark::<BlockHeader>(&name, &bytes),
            "AttestationData" => decode_and_benchmark::<AttestationData>(&name, &bytes),
            _ => unreachable!("supported_fixture filtered this name"),
        }
    }
}

fn decode_and_benchmark<T>(name: &str, bytes: &[u8])
where
    T: SszDecode + HashTreeRoot + LeanSszType,
{
    let value = T::from_ssz_bytes(bytes)
        .unwrap_or_else(|error| panic!("fixture/{name}: libssz decode failed: {error:?}"));
    benchmark(&format!("fixture/{name}"), &value);
}

fn from_hex(text: &str) -> Result<Vec<u8>, ()> {
    let text = text.strip_prefix("0x").unwrap_or(text);
    if !text.len().is_multiple_of(2) {
        return Err(());
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).map_err(|_| ()))
        .collect()
}
