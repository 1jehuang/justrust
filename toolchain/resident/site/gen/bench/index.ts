// The BenchHistory widget (renderer/core/src/render.rs embeds it; gen/build.ts bakes its HTML into the snapshot): /bench's tables and charts, rendered at build time from the benchmark
// history with the default choices, and the parsed history as JSON for client/bench.ts to redraw them from.
import { readdirSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { parseHistory } from './data';
import { initial, liveHtml, latestHtml, overTimeHtml } from './view';

/** docs/benchmarks/history at the repo root, or the directory BENCH_HISTORY names. Read on every build. */
const dir = () => (process.env.BENCH_HISTORY ? path.resolve(process.env.BENCH_HISTORY) : path.join(import.meta.dirname, '../../../docs/benchmarks/history'));

const section = (id: string, title: string, part: string, html: string) =>
	`<section id="${id}"><h2><a class="hash" href="#${id}">${title}</a></h2>\n<div data-part="${part}">${html}</div>\n</section>`;

export function benchHistory(): string {
	const d = dir();
	const files = readdirSync(d).filter((f) => f.endsWith('.jsonl')).sort().map((f) => readFileSync(path.join(d, f), 'utf8'));
	const data = parseHistory(files);
	const s = initial();
	const capped = [...data.build, ...data.live].some((m) => m.runs.length >= data.maxRuns);
	return [
		'<div class="bench">',
		section('latest', 'Latest run', 'latest', latestHtml(data, s)),
		section('time', 'Over time', 'time', overTimeHtml(data, s)),
		section('live', 'Live edits', 'live', liveHtml(data, s)),
		capped ? `<p class="dim note">The charts show the last ${data.maxRuns} runs of each machine.</p>` : '',
		// `<` escaped so that no string in the data can close the script element.
		`<script type="application/json" id="bench-data">${JSON.stringify(data).replace(/</g, '\\u003c')}</script>`,
		'</div>',
	].join('\n');
}
