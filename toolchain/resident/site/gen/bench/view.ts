// /bench as HTML: a pure function of the parsed history and what the reader picked. The build renders it once with
// the defaults (gen/bench/index.ts); client/bench.ts renders it again in the browser when a choice or the run under
// the pointer changes. No DOM or Node APIs here, so both can import it.
import { backendOrder, type Entry, type History, type Point, type Run } from './data';
import {
	BACKEND_NAMES, EDITS, EDIT_ORDER, MODES, PROGRAMS, REPO, SETUPS, SETUP_ORDER,
	commitUrl, day, megabytes, megabytesTick, millis, millisTick, projectName, seconds, secondsTick, short, speedup, when,
} from './format';
import { linearDomain, linearTicks, logDomain, logTicks } from './scale';

const LAST = Number.MAX_SAFE_INTEGER;

/** What the reader picked. A wanted value that the data does not have falls back, as the Svelte page did. */
export type State = {
	platform: string;
	summaryBackend: string;
	buildMachine: string;
	project: string;
	backend: string;
	active: number;
	liveMachine: string;
	liveActive: number;
};
export const initial = (): State => ({
	platform: 'Linux x86-64',
	summaryBackend: 'cranelift',
	buildMachine: '',
	project: 'ripgrep',
	backend: 'cranelift',
	active: LAST,
	liveMachine: '',
	liveActive: LAST,
});

/** A Choice button's click: `key` is its data-set. Picking a platform or machine starts at the latest run again. */
export function choose(s: State, key: string, value: string) {
	if (key === 'platform') Object.assign(s, { platform: value, buildMachine: '' });
	else if (key === 'buildMachine') Object.assign(s, { buildMachine: value, active: LAST });
	else if (key === 'liveMachine') Object.assign(s, { liveMachine: value, liveActive: LAST });
	else if (key === 'summaryBackend' || key === 'project' || key === 'backend') s[key] = value;
}

export const esc = (t: string | number) =>
	String(t).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const ranked = (order: string[]) => (a: string, b: string) =>
	(order.includes(a) ? order.indexOf(a) : order.length) - (order.includes(b) ? order.indexOf(b) : order.length) || a.localeCompare(b);
const unique = (items: string[]) => [...new Set(items)];
const projectOf = (key: string) => key.split('/')[0];
const backendOf = (key: string) => key.split('/')[1] ?? '';
type Option = { value: string; label: string };
const backendOptions = (keys: string[]): Option[] =>
	unique(keys.map(backendOf)).sort(backendOrder).map((b) => ({ value: b, label: BACKEND_NAMES[b] ?? b }));
const pickOr = (wanted: string, options: Option[]) => (options.some((o) => o.value === wanted) ? wanted : (options[0]?.value ?? ''));
const machineLabel = (name: string) => name || 'Unnamed machine';
const cpuLine = (run: Run) =>
	[run.cpu?.replace(/( \d+-Core)? Processor$/, ''), run.cores ? `${run.cores} cores` : undefined].filter(Boolean).join(', ');
const clamp = (i: number, n: number) => Math.max(0, Math.min(i, n - 1));
const commitLink = (commit: string) => `<a href="${esc(commitUrl(commit))}"><code>${esc(short(commit))}</code></a>`;

function choice(label: string, key: string, options: Option[], value: string) {
	if (options.length <= 1) return '';
	const buttons = options.map(
		(o) => `<button type="button" data-set="${key}" data-value="${esc(o.value)}" aria-pressed="${o.value === value}">${esc(o.label)}</button>`,
	);
	return `<div class="choice" role="group" aria-label="${esc(label)}">${buttons.join('')}</div>`;
}

// Charts.

export type Series = {
	label: string;
	/** blaze's numbers are drawn bright, the ones it is compared with dim. */
	tone: 'ours' | 'other';
	dashed?: boolean;
	values: (number | null | undefined)[];
	/** The fastest and slowest of the run's samples, drawn as a band. */
	lo?: (number | null | undefined)[];
	hi?: (number | null | undefined)[];
};
type ChartProps = {
	title: string;
	series: Series[];
	runs: Point[];
	active: number;
	/** The State field the pointer sets: every chart of a group shows the same run. */
	group: 'active' | 'liveActive';
	scale?: 'log' | 'linear';
	format: (v: number) => string;
	tick: (v: number) => string;
	/** A domain shared with other charts, so that they read alike. */
	domain?: [number, number] | null;
	empty?: string;
};

export const W = 340;
const H = 168;
export const M = { l: 52, r: 12, t: 10, b: 24 };
export const PW = W - M.l - M.r;
const PH = H - M.t - M.b;
const DOTS_UP_TO = 24;
const f1 = (v: number) => v.toFixed(1);

function chart({ title, series, runs, active, group, scale = 'log', format, tick, domain, empty = 'No data yet.' }: ChartProps) {
	const defined = (v: number | null | undefined): v is number =>
		typeof v === 'number' && Number.isFinite(v) && (scale === 'linear' || v > 0);
	const n = runs.length;
	if (!series.some((s) => s.values.some(defined))) {
		return `<figure class="chart"><figcaption><span class="title">${esc(title)}</span></figcaption><p class="empty">${esc(empty)}</p></figure>`;
	}
	// blaze's series last, so that it is drawn on top.
	const drawn = [...series].sort((a, b) => Number(a.tone === 'ours') - Number(b.tone === 'ours'));
	const all = series.flatMap((s) => [...s.values, ...(s.lo ?? []), ...(s.hi ?? [])]);
	const dom = domain ?? (scale === 'log' ? logDomain(all) : linearDomain(all)) ?? ([1, 10] as [number, number]);
	const ticks = scale === 'log' ? logTicks(dom) : linearTicks(dom);
	const x = (i: number) => (n <= 1 ? M.l + PW / 2 : M.l + (i / (n - 1)) * PW);
	const y = (v: number) => {
		const [lo, hi] = dom;
		const t = scale === 'log' ? (Math.log(v) - Math.log(lo)) / (Math.log(hi) - Math.log(lo)) : (v - lo) / (hi - lo);
		return M.t + PH - Math.min(1, Math.max(0, t)) * PH;
	};
	/** Runs of consecutive indices where `ok(i)`: a missing run breaks the line. */
	const stretches = (ok: (i: number) => boolean) => {
		const out: number[][] = [];
		let cur: number[] = [];
		for (let i = 0; i < n; i++) {
			if (ok(i)) cur.push(i);
			else if (cur.length) (out.push(cur), (cur = []));
		}
		if (cur.length) out.push(cur);
		return out;
	};
	const cls = (s: Series, base: string) => `${base} ${s.tone}${s.dashed ? ' dashed' : ''}`;

	const readout = series.map((s) => {
		const v = s.values[active];
		const key = `<svg class="key" viewBox="0 0 14 6" aria-hidden="true"><line x1="0" y1="3" x2="14" y2="3" class="${cls(s, 'stroke')}" /></svg>`;
		return `<span class="value">${key}${esc(s.label)} <b class="${s.tone}">${defined(v) ? esc(format(v)) : '–'}</b></span>`;
	});
	const run = runs[active];
	const valuetext = [
		run ? `${short(run.commit)}, ${day(run.date)}` : '',
		...series.map((s) => `${s.label} ${defined(s.values[active]) ? format(s.values[active] as number) : 'not measured'}`),
	].join(', ');

	const svg: string[] = [];
	for (const t of ticks) {
		svg.push(`<line class="gridline" x1="${M.l}" x2="${W - M.r}" y1="${f1(y(t))}" y2="${f1(y(t))}" />`);
		svg.push(`<text class="tick" x="${M.l - 7}" y="${f1(y(t))}" dy="0.32em" text-anchor="end">${esc(tick(t))}</text>`);
	}
	if (n > 1) {
		svg.push(`<text class="tick" x="${M.l}" y="${H - 6}" text-anchor="start">${esc(day(runs[0].date))}</text>`);
		svg.push(`<text class="tick" x="${W - M.r}" y="${H - 6}" text-anchor="end">${esc(day(runs[n - 1].date))}</text>`);
		svg.push(`<line class="guide" x1="${f1(x(active))}" x2="${f1(x(active))}" y1="${M.t}" y2="${M.t + PH}" />`);
	} else if (n === 1) {
		svg.push(`<text class="tick" x="${f1(x(0))}" y="${H - 6}" text-anchor="middle">${esc(day(runs[0].date))}</text>`);
	}
	for (const s of drawn) {
		const { lo, hi } = s;
		if (!lo || !hi) continue;
		for (const r of stretches((i) => defined(lo[i]) && defined(hi[i]))) {
			if (r.length > 1) {
				const top = r.map((i) => `${f1(x(i))},${f1(y(hi[i]!))}`);
				const bottom = [...r].reverse().map((i) => `${f1(x(i))},${f1(y(lo[i]!))}`);
				svg.push(`<path class="band ${s.tone}" d="M${top.join('L')}L${bottom.join('L')}Z" />`);
				continue;
			}
			const i = r[0];
			const [xi, yh, yl] = [x(i), y(hi[i]!), y(lo[i]!)];
			svg.push(`<line class="whisker ${s.tone}" x1="${f1(xi)}" x2="${f1(xi)}" y1="${f1(yh)}" y2="${f1(yl)}" />`);
			svg.push(`<line class="whisker ${s.tone}" x1="${f1(xi - 3)}" x2="${f1(xi + 3)}" y1="${f1(yh)}" y2="${f1(yh)}" />`);
			svg.push(`<line class="whisker ${s.tone}" x1="${f1(xi - 3)}" x2="${f1(xi + 3)}" y1="${f1(yl)}" y2="${f1(yl)}" />`);
		}
	}
	for (const s of drawn) {
		for (const st of stretches((i) => defined(s.values[i]))) {
			if (st.length < 2) continue;
			const d = st.map((i, k) => `${k ? 'L' : 'M'}${f1(x(i))},${f1(y(s.values[i]!))}`).join('');
			svg.push(`<path class="${cls(s, 'line')}" d="${d}" />`);
		}
	}
	for (const s of drawn) {
		s.values.forEach((v, i) => {
			if (!defined(v) || (n > DOTS_UP_TO && i !== active)) return;
			const r = runs[i];
			const on = i === active;
			const dot = `<circle class="dot ${s.tone}${on ? ' on' : ''}" cx="${f1(x(i))}" cy="${f1(y(v))}" r="${on ? 3.6 : 2.2}"><title>${esc(`${s.label} ${format(v)}, ${short(r.commit)}, ${day(r.date)}`)}</title></circle>`;
			svg.push(r.commit ? `<a href="${esc(commitUrl(r.commit))}" tabindex="-1" aria-hidden="true">${dot}</a>` : dot);
		});
	}
	const slider = `role="slider" tabindex="0" aria-label="${esc(`${title}: pick a run`)}" aria-valuemin="1" aria-valuemax="${n}" aria-valuenow="${active + 1}" aria-valuetext="${esc(valuetext)}"`;
	return `<figure class="chart"><figcaption><span class="title">${esc(title)}</span><span class="values">${readout.join('')}</span></figcaption><svg viewBox="0 0 ${W} ${H}" ${slider} data-group="${group}" data-n="${n}" data-active="${active}">${svg.join('')}</svg></figure>`;
}

function runLine(run: Point, index: number, count: number) {
	const commit = run.commit ? commitLink(run.commit) : 'unknown commit';
	const ci = run.runUrl ? `<span class="dim"> · </span><a href="${esc(run.runUrl)}">CI run</a>` : '';
	return `<p class="run"><span class="dim">Run ${index + 1} of ${count}:</span> ${commit} <span class="dim">on ${esc(when(run.date))}</span>${ci}</p>`;
}

// The latest run's tables.

type Row = {
	label: string;
	cargo?: number;
	ours?: number;
	cargoMem?: number;
	oursMem?: number;
	fallback: boolean;
	setups: Record<string, number>;
	fastest?: number;
};

function table(entry: Entry) {
	const phases = Object.keys(entry.phases).sort(ranked(EDIT_ORDER));
	const setups = unique(phases.flatMap((p) => Object.keys(entry.phases[p].setups ?? {}))).sort(ranked(SETUP_ORDER));
	const rows: Row[] = phases.map((p) => {
		const ph = entry.phases[p];
		const builds = [ph.cargo_s, ph.wall_s, ...Object.entries(ph.setups ?? {}).filter(([s]) => s !== 'check').map(([, v]) => v)];
		const known = builds.filter((v): v is number => v !== undefined);
		return {
			label: EDITS[p] ?? p,
			cargo: ph.cargo_s,
			ours: ph.wall_s,
			cargoMem: ph.cargo_mem_mb,
			oursMem: ph.mem_mb,
			fallback: p !== 'cold' && !!ph.built && ph.built !== 'daemon',
			setups: ph.setups ?? {},
			fastest: known.length ? Math.min(...known) : undefined,
		};
	});
	const memory = rows.some((r) => r.oursMem !== undefined || r.cargoMem !== undefined);
	return { setups, rows, memory };
}

/** The fastest way cargo built it, of plain cargo and the setups (not check, which builds no program). */
function fastestCargo(row: Row) {
	const all = [row.cargo, ...Object.entries(row.setups).filter(([s]) => s !== 'check').map(([, v]) => v)].filter((v): v is number => v !== undefined);
	return all.length ? Math.min(...all) : undefined;
}

const td = (text: string, cls = 'num', best = false) => `<td class="${cls}${best ? ' best' : ''}">${text}</td>`;
const th = (text: string) => `<th class="num">${text}</th>`;
const abbr = (title: string, text: string, cls = '') => `<abbr${cls ? ` class="${cls}"` : ''} title="${esc(title)}">${esc(text)}</abbr>`;

function tableHtml(entry: Entry) {
	const t = table(entry);
	const head = [
		'<th>Edit</th>',
		th('cargo'),
		th('blaze'),
		th('Faster'),
		...(t.memory
			? [th(abbr('The most every process doing the work held at once', 'cargo memory')), th(abbr('The daemon, its workers and the command, at their peak', 'blaze memory'))]
			: []),
		...t.setups.map((s) => th(abbr(SETUPS[s]?.[1] ?? s, SETUPS[s]?.[0] ?? s))),
		...(t.setups.length ? [th('vs fastest cargo')] : []),
	];
	const rows = t.rows.map((row) => {
		const mark = row.fallback ? abbr('The daemon handed this edit to cargo', ' (c)', 'mark') : '';
		const cells = [
			`<td>${esc(row.label)}</td>`,
			td(seconds(row.cargo), 'num', row.cargo === row.fastest),
			td(seconds(row.ours) + mark, 'num', row.ours === row.fastest),
			td(speedup(row.cargo, row.ours), 'num ratio'),
			...(t.memory ? [td(megabytes(row.cargoMem)), td(megabytes(row.oursMem))] : []),
			...t.setups.map((s) => td(seconds(row.setups[s]), 'num', s !== 'check' && row.setups[s] === row.fastest)),
			...(t.setups.length ? [td(speedup(fastestCargo(row), row.ours), 'num ratio')] : []),
		];
		return `<tr>${cells.join('')}</tr>`;
	});
	return `<div class="ref${t.setups.length ? ' sticky' : ''}"><table><thead><tr>${head.join('')}</tr></thead><tbody>${rows.join('')}</tbody></table></div>`;
}

export function latestHtml(data: History, s: State) {
	const latest = data.latest.find((l) => l.run.platform === s.platform) ?? data.latest.at(-1);
	if (!latest) return '<p class="dim">No benchmark runs recorded yet.</p>';
	const run = latest.run;
	const keys = Object.keys(latest.results);
	const backends = backendOptions(keys);
	const backend = pickOr(s.summaryBackend, backends);
	const shown = keys.filter((k) => backendOf(k) === backend);
	const setups = unique(shown.flatMap((k) => Object.values(latest.results[k].phases).flatMap((p) => Object.keys(p.setups ?? {})))).sort(ranked(SETUP_ORDER));
	const platforms = data.latest.map((l) => ({ value: l.run.platform, label: l.run.platform }));

	const out: string[] = [choice('Platform', 'platform', platforms, run.platform)];
	const cpu = cpuLine(run);
	out.push(
		`<p>${esc(run.platform)}, ${esc(machineLabel(run.machine ?? ''))}${cpu ? `&nbsp;(${esc(cpu)})` : ''}, blaze ` +
			`${run.commit ? commitLink(run.commit) : '(commit unknown)'} on ${esc(when(run.date))}${run.runs ? `, median of ${run.runs} edits` : ''}.` +
			`${run.runUrl ? ` <a href="${esc(run.runUrl)}">The CI run</a>.` : ''}</p>`,
	);
	out.push(choice('Backend', 'summaryBackend', backends, backend));
	for (const key of shown) {
		const entry = latest.results[key];
		out.push(`<h3>${esc(projectName(projectOf(key), entry.tag))}</h3>`, tableHtml(entry));
	}
	const notes = ['Bold is the fastest of each row.'];
	if (setups.length) {
		notes.push(
			`The columns after Faster are cargo on the same edits, set up other ways: ${esc(setups.map((x) => `${SETUPS[x]?.[0] ?? x} is ${SETUPS[x]?.[1] ?? x}`).join('; '))}.`,
		);
	}
	if (keys.some((k) => Object.entries(latest.results[k].phases).some(([p, ph]) => p !== 'cold' && ph.built && ph.built !== 'daemon'))) {
		notes.push("(c): the daemon handed the edit to cargo, so it took about cargo's time.");
	}
	out.push(`<p class="dim note">${notes.join(' ')}</p>`);
	return out.join('\n');
}

export function overTimeHtml(data: History, s: State) {
	const latest = data.latest.find((l) => l.run.platform === s.platform) ?? data.latest.at(-1);
	const machine =
		data.build.find((m) => m.machine === s.buildMachine) ??
		data.build.find((m) => m.machine === (latest?.run.machine ?? '')) ??
		data.build.at(-1);
	const runs = machine?.runs ?? [];
	if (!machine || !runs.length) return '<p class="dim">No benchmark runs recorded yet.</p>';
	const machines = data.build.map((m) => ({ value: m.machine, label: `${m.platform}: ${machineLabel(m.machine)}` }));
	const projects = unique(machine.keys.map(projectOf)).map((p) => ({ value: p, label: p }));
	const project = pickOr(s.project, projects);
	const backends = backendOptions(machine.keys.filter((k) => projectOf(k) === project));
	const backend = pickOr(s.backend, backends);
	const history = machine.results[`${project}/${backend}`];
	const active = clamp(s.active, runs.length);
	const phases = history?.phases ?? {};
	const name = esc(projectName(project, history?.tag));
	const c = { runs, active, group: 'active' as const };

	const edits = Object.keys(phases)
		.filter((p) => p !== 'cold')
		.sort(ranked(EDIT_ORDER))
		.map((p) => {
			const ph = phases[p];
			const series: Series[] = [
				{ label: 'blaze', tone: 'ours', values: ph.wall, lo: ph.min, hi: ph.max },
				{ label: 'cargo', tone: 'other', values: ph.cargo, lo: ph.cargoMin, hi: ph.cargoMax },
			];
			return { title: EDITS[p] ?? p, series };
		});
	const editDomain = logDomain(edits.flatMap((e) => e.series.flatMap((x) => [...x.values, ...(x.lo ?? []), ...(x.hi ?? [])])));
	const memory = Object.keys(phases)
		.sort(ranked(EDIT_ORDER))
		.filter((p) => phases[p].mem.length || phases[p].cargoMem.length)
		.map((p) => ({
			title: EDITS[p] ?? p,
			series: [
				{ label: 'blaze', tone: 'ours', values: phases[p].mem },
				{ label: 'cargo', tone: 'other', values: phases[p].cargoMem },
			] as Series[],
		}));
	const memoryDomain = logDomain(memory.flatMap((e) => e.series.flatMap((x) => x.values)));
	const cold = phases.cold;
	const mb = { format: megabytes, tick: megabytesTick, scale: 'linear' as const };

	const out = [
		choice('Machine', 'buildMachine', machines, machine.machine),
		choice('Project', 'project', projects, project),
		choice('Backend', 'backend', backends, backend),
		runLine(runs[active], active, runs.length),
		`<h3>${name}: edit, then build</h3>`,
		'<p class="dim note">Each point is a run\'s median, the band its fastest to slowest edit. Time is on a log scale, so equal steps are equal ratios. Point at a chart, or tap it, to read a run; click a point to open its commit.</p>',
		`<div class="charts">${edits.map((e) => chart({ ...c, ...e, format: seconds, tick: secondsTick, domain: editDomain })).join('')}</div>`,
	];
	if (memory.length) {
		out.push(
			`<h3>${name}: memory while it builds</h3>`,
			`<p class="dim note">The most every process doing the work held at once: for blaze the daemon, its workers and the command; for plain cargo, cargo itself, rustc and the linker (<a href="${REPO}/blob/main/docs/benchmarks.md#memory-and-disk">how</a>). Log scale.</p>`,
			`<div class="charts">${memory.map((e) => chart({ ...c, ...e, format: megabytes, tick: megabytesTick, domain: memoryDomain })).join('')}</div>`,
		);
	}
	const measured = 'Not measured in these runs yet.';
	const coldSeries: Series[] = [
		{ label: 'blaze', tone: 'ours', values: cold?.wall ?? [] },
		...(cold?.warmStore.some((v) => v !== null) ? [{ label: 'blaze, warm store', tone: 'ours' as const, dashed: true, values: cold.warmStore }] : []),
		{ label: 'cargo', tone: 'other', values: cold?.cargo ?? [] },
	];
	const store = [
		chart({ ...c, title: 'Cold build: cargo, and blaze until ready', series: coldSeries, scale: 'linear', format: seconds, tick: secondsTick }),
		chart({
			...c, ...mb, title: 'Daemon between edits', empty: measured,
			series: [
				{ label: 'after the edits', tone: 'ours', values: history?.residentEnd ?? [] },
				{ label: 'once ready', tone: 'other', dashed: true, values: history?.residentReady ?? [] },
			],
		}),
		chart({
			...c, ...mb, title: 'On disk', empty: measured,
			series: [
				{ label: 'blaze: target, store, home', tone: 'ours', values: history?.disk ?? [] },
				{ label: 'cargo: target', tone: 'other', values: history?.cargoDisk ?? [] },
			],
		}),
		chart({ ...c, ...mb, title: 'Daemon peak memory', series: [{ label: 'blaze', tone: 'ours', values: history?.memory ?? [] }] }),
		chart({ ...c, ...mb, title: 'Store size', empty: measured, series: [{ label: 'blaze', tone: 'ours', values: history?.store ?? [] }] }),
	];
	out.push('<h3>Cold build, memory and store</h3>', `<div class="charts">${store.join('')}</div>`);
	return out.join('\n');
}

export function liveHtml(data: History, s: State) {
	const machine = data.live.find((m) => m.machine === s.liveMachine) ?? data.live.at(-1);
	const runs = machine?.runs ?? [];
	if (!machine || !runs.length) return '<p class="dim">No live-edit runs recorded yet.</p>';
	const active = clamp(s.liveActive, runs.length);
	const out = [
		`<p>Edit to running: from the edit leaving the editor to the new code running in the program, on ${esc(machineLabel(machine.machine))}. Solid is the median edit, dashed the 90th percentile.</p>`,
		choice('Machine', 'liveMachine', data.live.map((m) => ({ value: m.machine, label: machineLabel(m.machine) })), machine.machine),
		runLine(runs[active], active, runs.length),
	];
	for (const program of machine.programs) {
		const charts = machine.modes[program].map((mode) => {
			const stats = machine.results[program][mode];
			return chart({
				title: MODES[mode] ?? mode, runs, active, group: 'liveActive', format: millis, tick: millisTick,
				series: [
					{ label: 'median', tone: 'ours', values: stats.p50 },
					{ label: '90th percentile', tone: 'other', dashed: true, values: stats.p90 },
				],
			});
		});
		out.push(`<h3>${esc(PROGRAMS[program] ?? program)}</h3>`, `<div class="charts">${charts.join('')}</div>`);
	}
	return out.join('\n');
}

/** The parts of the page client/bench.ts redraws, by their data-part. */
export const PARTS = { latest: latestHtml, time: overTimeHtml, live: liveHtml };
/** Which part a chart group lives in. */
export const GROUP_PART = { active: 'time', liveActive: 'live' } as const;
