// Numbers as the README writes them (harness/benchreport.py `seconds` and `speedup`).

export const REPO = 'https://github.com/indexable-inc/blaze';

export const commitUrl = (commit: string) => `${REPO}/commit/${commit}`;
export const short = (commit: string | undefined) => (commit ?? 'unknown').slice(0, 10);

export function seconds(v: number | null | undefined) {
	if (v == null) return '–';
	return v < 1 ? `${Math.round(v * 1000)} ms` : `${v.toFixed(2)} s`;
}

/** cargo's time over blaze's: "5.0x", "14x", or "2.0x slower" when blaze took longer. */
export function speedup(cargo: number | undefined, ours: number | undefined) {
	if (!cargo || !ours) return '';
	const ratio = cargo / ours;
	const times = (r: number) => (r >= 10 ? `${r.toFixed(0)}x` : `${r.toFixed(1)}x`);
	return ratio >= 0.95 ? times(ratio) : `${times(1 / ratio)} slower`;
}

export function millis(v: number | null | undefined) {
	if (v == null) return '–';
	if (v >= 1000) return seconds(v / 1000);
	return v < 10 ? `${v.toFixed(1)} ms` : `${Math.round(v)} ms`;
}

export function megabytes(v: number | null | undefined) {
	if (v == null) return '–';
	return v >= 1000 ? `${(v / 1000).toFixed(2)} GB` : `${Math.round(v)} MB`;
}

const trim = (v: number) => String(Number(v.toPrecision(3)));

/** Axis labels: as short as the value allows. */
export const secondsTick = (v: number) => (v === 0 ? '0' : v < 1 ? `${trim(v * 1000)} ms` : `${trim(v)} s`);
export const millisTick = (v: number) => (v === 0 ? '0' : v >= 1000 ? `${trim(v / 1000)} s` : `${trim(v)} ms`);
export const megabytesTick = (v: number) => (v === 0 ? '0' : v >= 1000 ? `${trim(v / 1000)} GB` : `${trim(v)} MB`);

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/** "Oct 20", in UTC so that the prerendered page and the browser agree. */
export function day(iso: string) {
	const d = new Date(iso);
	return Number.isNaN(d.getTime()) ? '' : `${MONTHS[d.getUTCMonth()]} ${d.getUTCDate()}`;
}

/** "2026-10-20 16:53 UTC". */
export function when(iso: string) {
	const d = new Date(iso);
	return Number.isNaN(d.getTime()) ? iso : `${d.toISOString().slice(0, 10)} ${d.toISOString().slice(11, 16)} UTC`;
}

export const EDITS: Record<string, string> = {
	body: 'Edit a function',
	leaf: 'Edit a widely used function',
	signature: 'Change a signature',
	generic: 'Edit a generic function',
	newfn: 'Add a function',
	struct: 'Add a struct field',
	'no-op': 'Nothing changed',
	'test-edit': 'Edit, then run the tests',
	test: 'Run the tests, nothing changed',
	cold: 'Cold build, until ready',
};
export const EDIT_ORDER = ['body', 'leaf', 'signature', 'generic', 'newfn', 'struct', 'no-op', 'test-edit', 'test', 'cold'];

/** The other cargo setups measured on the same edits: a short column name and what it is. */
export const SETUPS: Record<string, [string, string]> = {
	mold: ['mold', 'cargo with the mold linker'],
	wild: ['wild', 'cargo with the wild linker'],
	threads8: ['8 threads', 'cargo with -Zthreads=8, the parallel front end'],
	nightly: ['nightly', 'cargo on plain nightly Rust'],
	cranelift: ['Cranelift', 'cargo on nightly with the Cranelift backend'],
	fastest: ['fastest', 'cargo with Cranelift, wild and 8 threads together'],
	check: ['check', 'cargo check: no program is built'],
};
export const SETUP_ORDER = ['mold', 'wild', 'threads8', 'nightly', 'cranelift', 'fastest', 'check'];

export const BACKEND_NAMES: Record<string, string> = { cranelift: 'Cranelift', llvm: 'LLVM' };

export const PROGRAMS: Record<string, string> = { probe: 'probe, a small program', life: 'Game of Life' };
export const MODES: Record<string, string> = {
	socket: 'One edit, sent to the daemon',
	burst: 'Typing, keystroke to running',
	replay: 'Replayed edits',
	cargo: 'Save, then cargo blaze build',
};

export const projectName = (project: string, tag: string | undefined) =>
	`${project} ${(tag ?? '').replace(/^axum-v/, '').replace(/^v/, '')}`.trim();
