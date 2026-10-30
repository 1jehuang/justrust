// The benchmark history (docs/benchmarks/history/*.jsonl, written by harness/benchhistory.py), parsed at build time
// into what /bench draws. Any key of a line but `suite`, `date` and `results` may be missing.

export type Phase = {
	wall_s?: number;
	cargo_s?: number;
	min_s?: number;
	max_s?: number;
	cargo_min_s?: number;
	cargo_max_s?: number;
	warm_store_s?: number;
	rss_mb?: number;
	/** The most every process doing the work held at once (harness/memory.py): blaze's, and cargo's. */
	mem_mb?: number;
	cargo_mem_mb?: number;
	/** What the daemon held once the phase was over. */
	resident_mb?: number;
	built?: string;
	setups?: Record<string, number>;
};
export type Entry = {
	tag?: string;
	store_mb?: number;
	resident_ready_mb?: number;
	resident_end_mb?: number;
	builds?: number;
	disk_mb?: number;
	cargo_disk_mb?: number;
	phases: Record<string, Phase>;
};
export type Run = {
	date: string;
	commit?: string;
	runUrl?: string;
	platform: string;
	machine?: string;
	cpu?: string;
	cores?: number;
	runs?: number;
};
/** One number per run, oldest first; `null` where the run did not measure it, and empty when none did. */
export type Column = (number | null)[];
export type PhaseHistory = {
	wall: Column;
	cargo: Column;
	min: Column;
	max: Column;
	cargoMin: Column;
	cargoMax: Column;
	/** Cold only: until ready in another checkout, with the store the first one filled. */
	warmStore: Column;
	mem: Column;
	cargoMem: Column;
	resident: Column;
};
/** The history of one `project/backend` on one machine. */
export type ProjectHistory = {
	tag?: string;
	memory: Column;
	store: Column;
	/** The daemon once ready, and after the run's edits. */
	residentReady: Column;
	residentEnd: Column;
	/** On disk: blaze's target directory, store and home; cargo's target directory. */
	disk: Column;
	cargoDisk: Column;
	phases: Record<string, PhaseHistory>;
};
export type Point = { date: string; commit?: string; runUrl?: string };
export type BuildMachine = { machine: string; platform: string; runs: Point[]; keys: string[]; results: Record<string, ProjectHistory> };
/** The newest run of one platform. */
export type Latest = { run: Run; results: Record<string, Entry> };
export type LiveMachine = {
	machine: string;
	runs: Point[];
	programs: string[];
	modes: Record<string, string[]>;
	results: Record<string, Record<string, { p50: Column; p90: Column }>>;
};
export type History = {
	build: BuildMachine[];
	live: LiveMachine[];
	/** The newest run of each platform, in PLATFORMS order. */
	latest: Latest[];
	/** The charts keep the most recent runs of each machine. */
	maxRuns: number;
};

const MAX_RUNS = 120;
const PHASE_NUMBERS = [
	'wall_s', 'cargo_s', 'min_s', 'max_s', 'cargo_min_s', 'cargo_max_s', 'warm_store_s', 'rss_mb', 'mem_mb', 'cargo_mem_mb', 'resident_mb',
] as const;
const ENTRY_NUMBERS = ['store_mb', 'resident_ready_mb', 'resident_end_mb', 'builds', 'disk_mb', 'cargo_disk_mb'] as const;

// The platforms each benchmark runs on, in the README's order (harness/benchplatforms.py PLATFORMS).
export const PLATFORMS = ['macOS arm64', 'Linux aarch64', 'Linux x86-64'];
// The README's order of projects (harness/benchreport.py PROJECTS) and of backends.
const PROJECTS = ['ripgrep', 'fd', 'bat', 'hyperfine', 'just', 'axum', 'nushell', 'helix', 'bevy', 'biome', 'zed', 'cargo', 'bun'];
const BACKENDS = ['cranelift', 'llvm'];
const PROGRAMS = ['probe', 'life'];
const MODES = ['socket', 'burst', 'replay', 'cargo'];

type Json = Record<string, unknown>;

const isObject = (v: unknown): v is Json => typeof v === 'object' && v !== null && !Array.isArray(v);
const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
const str = (v: unknown) => (typeof v === 'string' && v !== '' ? v : undefined);

function ranked(order: string[]) {
	const rank = (name: string) => (order.includes(name) ? order.indexOf(name) : order.length);
	return (a: string, b: string) => rank(a) - rank(b) || a.localeCompare(b);
}

export function projectOrder(a: string, b: string) {
	const [pa, ba] = a.split('/');
	const [pb, bb] = b.split('/');
	return ranked(PROJECTS)(pa, pb) || ranked(BACKENDS)(ba ?? '', bb ?? '');
}
export const backendOrder = ranked(BACKENDS);
export const platformOrder = ranked(PLATFORMS);

/** A line's platform: its own, or for a line from before lines said it, what its CPU says. */
function platformOf(line: Json) {
	const named = str(line.platform);
	if (named) return named;
	const cpu = str(line.cpu) ?? '';
	if (/Apple/.test(cpu)) return 'macOS arm64';
	if (/Neoverse|Graviton|aarch64|ARM/i.test(cpu)) return 'Linux aarch64';
	return 'Linux x86-64';
}

function run(line: Json): Run {
	return {
		date: str(line.date) ?? '',
		commit: str(line.commit),
		runUrl: runUrl(line.run_url),
		platform: platformOf(line),
		machine: str(line.machine),
		cpu: str(line.cpu),
		cores: num(line.cores),
		runs: num(line.runs),
	};
}

function entry(value: unknown): Entry | null {
	if (!isObject(value) || !isObject(value.phases)) return null;
	const phases: Record<string, Phase> = {};
	for (const [label, raw] of Object.entries(value.phases)) {
		if (!isObject(raw)) continue;
		const phase: Phase = {};
		for (const field of PHASE_NUMBERS) {
			const v = num(raw[field]);
			if (v !== undefined) phase[field] = v;
		}
		const built = str(raw.built);
		if (built) phase.built = built;
		if (isObject(raw.setups)) {
			const setups: Record<string, number> = {};
			for (const [name, seconds] of Object.entries(raw.setups)) {
				const v = num(seconds);
				if (v !== undefined) setups[name] = v;
			}
			if (Object.keys(setups).length) phase.setups = setups;
		}
		phases[label] = phase;
	}
	const e: Entry = { tag: str(value.tag), phases };
	for (const field of ENTRY_NUMBERS) {
		const v = num(value[field]);
		if (v !== undefined) e[field] = v;
	}
	return e;
}

function column(values: (number | undefined)[]): Column {
	return values.some((v) => v !== undefined) ? values.map((v) => v ?? null) : [];
}

/** A run's CI link; runs from before the rename name the repo indexable-inc/ra-jit, which GitHub redirects to blaze. */
const runUrl = (v: unknown): string | undefined => str(v)?.replace('github.com/indexable-inc/ra-jit/', 'github.com/indexable-inc/blaze/');
const point = (line: Json): Point => ({ date: str(line.date) ?? '', commit: str(line.commit), runUrl: runUrl(line.run_url) });

/** The lines of every file, in file order, that are an object with a known suite and results. */
function lines(files: string[]): Json[] {
	const out: Json[] = [];
	for (const text of files) {
		for (const raw of text.split('\n')) {
			if (!raw.trim()) continue;
			try {
				const line = JSON.parse(raw);
				if (isObject(line) && (line.suite === 'build' || line.suite === 'live') && isObject(line.results)) out.push(line);
			} catch {
				// A line that is not JSON (a merge gone wrong, a cut-off write) is left out.
			}
		}
	}
	return out;
}

function byDate(a: Json, b: Json) {
	const ta = Date.parse(str(a.date) ?? '');
	const tb = Date.parse(str(b.date) ?? '');
	return (Number.isNaN(ta) ? 0 : ta) - (Number.isNaN(tb) ? 0 : tb);
}

/** Lines grouped by machine (numbers from two machines never share a series), each oldest first. */
function machines(all: Json[]) {
	const groups = new Map<string, Json[]>();
	for (const line of [...all].sort(byDate)) {
		const name = str(line.machine) ?? '';
		groups.set(name, [...(groups.get(name) ?? []), line]);
	}
	return [...groups].map(([machine, group]) => ({ machine, group: group.slice(-MAX_RUNS) }));
}

export function parseHistory(files: string[]): History {
	const all = lines(files);
	const builds = all.filter((l) => l.suite === 'build');
	const lives = all.filter((l) => l.suite === 'live');

	const build: BuildMachine[] = machines(builds).map(({ machine, group }) => {
		const keys = [...new Set(group.flatMap((l) => Object.keys(l.results as Json)))].sort(projectOrder);
		const results: BuildMachine['results'] = {};
		for (const key of keys) {
			const entries = group.map((l) => entry((l.results as Json)[key]));
			const labels = [...new Set(entries.flatMap((e) => Object.keys(e?.phases ?? {})))];
			const phases: ProjectHistory['phases'] = {};
			for (const label of labels) {
				const at = (field: keyof Phase) => column(entries.map((e) => e?.phases[label]?.[field] as number | undefined));
				phases[label] = {
					wall: at('wall_s'),
					cargo: at('cargo_s'),
					min: at('min_s'),
					max: at('max_s'),
					cargoMin: at('cargo_min_s'),
					cargoMax: at('cargo_max_s'),
					warmStore: at('warm_store_s'),
					mem: at('mem_mb'),
					cargoMem: at('cargo_mem_mb'),
					resident: at('resident_mb'),
				};
			}
			const memory = entries.map((e) => {
				const rss = Object.values(e?.phases ?? {}).flatMap((p) => (p.rss_mb === undefined ? [] : [p.rss_mb]));
				return rss.length ? Math.max(...rss) : undefined;
			});
			results[key] = {
				tag: entries.findLast((e) => e?.tag)?.tag,
				memory: column(memory),
				store: column(entries.map((e) => e?.store_mb)),
				residentReady: column(entries.map((e) => e?.resident_ready_mb)),
				residentEnd: column(entries.map((e) => e?.resident_end_mb)),
				disk: column(entries.map((e) => e?.disk_mb)),
				cargoDisk: column(entries.map((e) => e?.cargo_disk_mb)),
				phases,
			};
		}
		return { machine, platform: platformOf(group.at(-1)!), runs: group.map(point), keys, results };
	}).sort((a, b) => platformOrder(a.platform, b.platform));

	const live: LiveMachine[] = machines(lives).map(({ machine, group }) => {
		const programs = [...new Set(group.flatMap((l) => Object.keys(l.results as Json)))].sort(ranked(PROGRAMS));
		const modes: LiveMachine['modes'] = {};
		const results: LiveMachine['results'] = {};
		for (const program of programs) {
			const perRun = group.map((l) => (l.results as Json)[program]);
			modes[program] = [...new Set(perRun.flatMap((p) => (isObject(p) ? Object.keys(p) : [])))].sort(ranked(MODES));
			results[program] = {};
			for (const mode of modes[program]) {
				const stat = (field: string) => column(perRun.map((p) => (isObject(p) && isObject(p[mode]) ? num(p[mode][field]) : undefined)));
				results[program][mode] = { p50: stat('p50_ms'), p90: stat('p90_ms') };
			}
		}
		return { machine, runs: group.map(point), programs, modes, results };
	});

	const newest = new Map<string, Json>();
	for (const line of [...builds].sort(byDate)) newest.set(platformOf(line), line);
	const latest: Latest[] = [...newest]
		.sort(([a], [b]) => platformOrder(a, b))
		.map(([, last]) => {
			const results: Record<string, Entry> = {};
			for (const key of Object.keys(last.results as Json).sort(projectOrder)) {
				const e = entry((last.results as Json)[key]);
				if (e) results[key] = e;
			}
			return { run: run(last), results };
		});
	return { build, live, latest, maxRuns: MAX_RUNS };
}
