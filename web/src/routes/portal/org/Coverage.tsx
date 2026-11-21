import { createMemo, createSignal, For, type JSX, onCleanup, onMount, Show } from "solid-js";
import { scaleUtc } from "d3-scale";
import { utcMonth } from "d3-time";
import { formatDate, formatPeriod, type UploadSummary } from "~/lib/statements";

/**
 * Which dates an account's saved statements cover, on one SVG timeline: a block per saved
 * statement (click one to open it; overlapping ones stack into another row), and the gaps
 * *between statements* as hatched bands through the whole chart. Each
 * statement counts as covering every day from its first to its last row, so quiet spells
 * inside a statement are never gaps. With `pending`, the statement being uploaded gets its
 * own row, so you can see where it lands before saving it.
 */

const DAY_MS = 86_400_000;

/** Days since 1970-01-01 for a `YYYY-MM-DD` date (UTC, so no daylight-saving drift). */
const toDay = (iso: string) => {
  const [y, m, d] = iso.slice(0, 10).split("-").map(Number);
  return Math.round(Date.UTC(y, m - 1, d) / DAY_MS);
};
const toIso = (day: number) => new Date(day * DAY_MS).toISOString().slice(0, 10);

/** Inclusive day range. */
type Span = { from: number; to: number };

type Block = Span & { upload: UploadSummary };

/** Overlapping or touching spans merged (a statement ending the day before the next starts leaves no gap). */
function union(spans: Span[]): Span[] {
  const sorted = [...spans].sort((a, b) => a.from - b.from);
  const out: Span[] = [];
  for (const s of sorted) {
    const last = out[out.length - 1];
    if (last && s.from <= last.to + 1) last.to = Math.max(last.to, s.to);
    else out.push({ from: s.from, to: s.to });
  }
  return out;
}

/** The days between merged spans. */
const gapsOf = (merged: Span[]): Span[] => merged.slice(1).map((s, i) => ({ from: merged[i].to + 1, to: s.from - 1 }));

/** Days of `span` inside any of `spans` (which don't overlap each other). */
const daysWithin = (span: Span, spans: Span[]) =>
  spans.reduce((n, s) => n + Math.max(0, Math.min(span.to, s.to) - Math.max(span.from, s.from) + 1), 0);

const days = (s: Span) => s.to - s.from + 1;
const dayCount = (n: number) => `${n.toLocaleString()} ${n === 1 ? "day" : "days"}`;
const spanText = (s: Span) => formatPeriod(toIso(s.from), toIso(s.to));

/** Rows of blocks, each statement in the first row where it doesn't overlap the one before. */
function lanes(blocks: Block[]): Block[][] {
  const rows: Block[][] = [];
  for (const b of [...blocks].sort((x, y) => x.from - y.from || y.to - x.to)) {
    const row = rows.find((r) => r[r.length - 1].to < b.from);
    if (row) row.push(b);
    else rows.push([b]);
  }
  return rows;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

// Layout, in px.
const LABEL_W = 72; // row labels on the left
const RIGHT = 8;
const ROW_H = 20;
const ROW_GAP = 6;
const AXIS_H = 22;
/** Smallest hover target for a gap or block, however few days it spans. */
const MIN_HIT = 10;

type Tip = { x: number; y: number; body: JSX.Element };

export function Coverage(props: {
  uploads: UploadSummary[];
  /** The statement being uploaded: its period and a name for it. */
  pending?: { label: string; first_date: string | null; last_date: string | null };
  /** Clicking a saved statement (left out in the staging area, where it would lose the upload). */
  onOpen?: (upload: UploadSummary) => void;
}) {
  let wrap!: HTMLDivElement;
  const [width, setWidth] = createSignal(0);
  const [tip, setTip] = createSignal<Tip | null>(null);
  onMount(() => {
    const ro = new ResizeObserver(([e]) => setWidth(e.contentRect.width));
    ro.observe(wrap);
    onCleanup(() => ro.disconnect());
  });
  // Unique per chart, for the hatch pattern and clip paths.
  const uid = `cov${Math.random().toString(36).slice(2, 8)}`;

  const data = createMemo(() => {
    const blocks: Block[] = props.uploads
      .filter((u) => u.first_date && u.last_date)
      .map((u) => {
        const a = toDay(u.first_date!);
        const b = toDay(u.last_date!);
        return { from: Math.min(a, b), to: Math.max(a, b), upload: u };
      });
    const p = props.pending;
    const pending: Span | null =
      p?.first_date && p.last_date
        ? { from: Math.min(toDay(p.first_date), toDay(p.last_date)), to: Math.max(toDay(p.first_date), toDay(p.last_date)) }
        : null;
    const everything: Span[] = pending ? [...blocks, pending] : blocks;
    if (everything.length === 0) return null;
    const saved = union(blocks);
    const covered = union(everything);
    // What's either side of a gap, or overlapping a statement, by name.
    const named = [
      ...blocks.map((b) => ({ span: b as Span, name: b.upload.filename })),
      ...(pending ? [{ span: pending, name: "this statement" }] : []),
    ];
    const gaps = gapsOf(covered).map((g) => ({
      ...g,
      before: named.filter((n) => n.span.to === g.from - 1).map((n) => n.name),
      after: named.filter((n) => n.span.from === g.to + 1).map((n) => n.name),
    }));
    return {
      from: Math.min(...everything.map((s) => s.from)),
      // The day after the last one, so the last day (and one-day statements) get a width.
      to: Math.max(...everything.map((s) => s.to)) + 1,
      lanes: lanes(blocks),
      gaps,
      /** The other statements `s` shares days with, and how many. */
      overlapping: (s: Span) =>
        named
          .filter((n) => n.span !== s)
          .map((n) => ({ name: n.name, days: Math.min(s.to, n.span.to) - Math.max(s.from, n.span.from) + 1 }))
          .filter((n) => n.days > 0),
      pending,
      // What the pending statement does: gap days it fills, days already covered, days it adds outside.
      effect: pending && {
        fills: daysWithin(pending, gapsOf(saved)),
        overlap: daysWithin(pending, saved),
        extends:
          saved.length === 0 ? days(pending) : days(pending) - daysWithin(pending, [{ from: saved[0].from, to: saved[saved.length - 1].to }]),
        first: saved.length === 0,
      },
      undated: props.uploads.length - blocks.length,
    };
  });

  // Hover/focus: the tooltip sits above the pointer (or the focused mark), inside the chart.
  const showTip = (e: MouseEvent | FocusEvent, body: JSX.Element) => {
    const box = wrap.getBoundingClientRect();
    if (e instanceof MouseEvent) {
      setTip({ x: e.clientX - box.left, y: e.clientY - box.top, body });
    } else {
      const r = (e.currentTarget as Element).getBoundingClientRect();
      setTip({ x: r.left + r.width / 2 - box.left, y: r.top - box.top, body });
    }
  };
  const hideTip = () => setTip(null);

  return (
    <div class="coverage-viz flex flex-col gap-2">
      <div ref={wrap} class="relative w-full" onMouseLeave={hideTip}>
        <Show when={data() && width() > LABEL_W + RIGHT + 40 ? data() : undefined}>
          {(d) => {
            const x = createMemo(() =>
              scaleUtc()
                .domain([new Date(d().from * DAY_MS), new Date(d().to * DAY_MS)])
                .range([LABEL_W, width() - RIGHT]),
            );
            const xd = (day: number) => x()(new Date(day * DAY_MS));
            /** Left edge and width of a day span, at least `min` px wide (centred). */
            const box = (s: Span, min = 0) => {
              const l = xd(s.from);
              const w = xd(s.to + 1) - l;
              return w >= min ? { x: l, w } : { x: l + w / 2 - min / 2, w: min };
            };
            const pendingY = 0;
            const lanesY = () => pendingY + (d().pending ? ROW_H + ROW_GAP : 0);
            const plotBottom = () => lanesY() + d().lanes.length * (ROW_H + ROW_GAP) - ROW_GAP;
            const height = () => plotBottom() + AXIS_H;
            const ticks = createMemo(() => {
              // Every month if the labels fit (about 44px each), else every 3rd, 6th or 12th.
              const perMonth = (width() - LABEL_W - RIGHT) / ((d().to - d().from) / 30.4);
              const step = [1, 3, 6].find((n) => perMonth * n >= 44) ?? 12;
              return x()
                .ticks(utcMonth.every(step)!)
                .filter((t) => t.getTime() > d().from * DAY_MS && t.getTime() < d().to * DAY_MS)
                .map((t, i) => {
                  const m = t.getUTCMonth();
                  return { x: x()(t), label: m === 0 || step === 12 || i === 0 ? `${MONTHS[m]} ${t.getUTCFullYear()}` : MONTHS[m] };
                });
            });

            return (
              <svg width={width()} height={height()} class="block overflow-visible text-foreground" role="group" aria-label="Statement coverage timeline">
                <defs>
                  {/* Long names fade out at the block's end rather than being cut mid-letter. */}
                  <linearGradient id={`${uid}-fade`} x1="0" x2="1" y1="0" y2="0">
                    <stop offset="0.8" stop-color="white" />
                    <stop offset="1" stop-color="white" stop-opacity="0" />
                  </linearGradient>
                  <pattern id={`${uid}-hatch`} width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                    <rect width="6" height="6" fill="var(--viz-gap)" fill-opacity="0.12" />
                    <line x1="0" y1="0" x2="0" y2="6" stroke="var(--viz-gap)" stroke-width="2" stroke-opacity="0.7" />
                  </pattern>
                </defs>

                {/* Row labels. */}
                <g class="fill-muted-foreground text-[11px]">
                  <Show when={d().pending}>
                    <text x={0} y={pendingY + ROW_H / 2} dominant-baseline="central" class="fill-foreground font-medium">
                      This one
                    </text>
                  </Show>
                  <Show when={d().lanes.length > 0}>
                    <text x={0} y={lanesY() + ROW_H / 2} dominant-baseline="central">Coverage</text>
                  </Show>
                </g>

                {/* Month gridlines, recessive. */}
                <g>
                  <For each={ticks()}>
                    {(t) => <line x1={t.x} x2={t.x} y1={0} y2={plotBottom()} class="stroke-border" stroke-width="1" />}
                  </For>
                </g>

                {/* Gaps between statements: hatched bands through the whole plot. */}
                <For each={d().gaps}>
                  {(g) => {
                    const b = () => box(g, 2);
                    const body = (
                      <>
                        <p class="font-semibold">{dayCount(days(g))} with no statement</p>
                        <p class="text-muted-foreground">{spanText(g)}</p>
                        <Show when={g.before.length > 0}>
                          <p class="mt-1">After {g.before.join(", ")}</p>
                        </Show>
                        <Show when={g.after.length > 0}>
                          <p>Before {g.after.join(", ")}</p>
                        </Show>
                      </>
                    );
                    return (
                      <g
                        tabindex="0"
                        aria-label={`Gap: ${dayCount(days(g))}, ${spanText(g)}`}
                        class="outline-none"
                        onMouseMove={(e) => showTip(e, body)}
                        onFocus={(e) => showTip(e, body)}
                        onBlur={hideTip}
                      >
                        <rect x={b().x} y={0} width={b().w} height={plotBottom()} fill={`url(#${uid}-hatch)`} />
                        {/* Bigger than the band, so a one-day gap is still easy to hover. */}
                        <rect x={box(g, MIN_HIT).x} y={0} width={box(g, MIN_HIT).w} height={plotBottom()} fill="transparent" />
                      </g>
                    );
                  }}
                </For>

                {/* The statement being uploaded. */}
                <Show when={d().pending}>
                  {(s) => {
                    const b = () => box(s(), 3);
                    const body = (
                      <>
                        <p class="font-semibold">{spanText(s())}</p>
                        <p>{props.pending!.label}</p>
                        <p class="text-muted-foreground">Not saved yet</p>
                        <Overlaps list={d().overlapping(s())} />
                      </>
                    );
                    return (
                      <g onMouseMove={(e) => showTip(e, body)}>
                        <clipPath id={`${uid}-pending`}>
                          <rect x={b().x} y={pendingY} width={b().w} height={ROW_H} />
                        </clipPath>
                        <rect
                          x={b().x + 1}
                          y={pendingY + 1}
                          width={Math.max(1, b().w - 2)}
                          height={ROW_H - 2}
                          rx="2"
                          fill="var(--viz-pending)"
                          fill-opacity="0.25"
                          stroke="var(--viz-pending)"
                          stroke-width="1.5"
                          stroke-dasharray="4 3"
                        />
                        <Show when={b().w > 48}>
                          <text
                            x={b().x + 6}
                            y={pendingY + ROW_H / 2}
                            dominant-baseline="central"
                            clip-path={`url(#${uid}-pending)`}
                            class="fill-foreground text-[11px] font-medium"
                          >
                            {props.pending!.label}
                          </text>
                        </Show>
                      </g>
                    );
                  }}
                </Show>

                {/* Saved statements: a square-cornered block each, 2px apart; click to open. */}
                <For each={d().lanes}>
                  {(row, i) => (
                    <For each={row}>
                      {(b, j) => {
                        const u = b.upload;
                        const y = () => lanesY() + i() * (ROW_H + ROW_GAP);
                        const r = () => box(b, 3);
                        const id = () => `${uid}-b${i()}-${j()}`;
                        const body = (
                          <>
                            <p class="font-semibold">{formatPeriod(u.first_date, u.last_date)}</p>
                            <p>{u.filename}</p>
                            <p class="text-muted-foreground">
                              {u.rows_parsed.toLocaleString()} rows · {dayCount(days(b))}
                            </p>
                            <p class="text-muted-foreground">
                              Saved {formatDate(u.uploaded_at)}
                              {u.uploaded_by ? ` by ${u.uploaded_by}` : ""}
                            </p>
                            <Overlaps list={d().overlapping(b)} />
                            <Show when={props.onOpen}>
                              <p class="mt-1 text-muted-foreground">Click to open</p>
                            </Show>
                          </>
                        );
                        const open = () => props.onOpen?.(u);
                        return (
                          <g
                            class={props.onOpen ? "group cursor-pointer outline-none" : "group outline-none"}
                            tabindex={props.onOpen ? "0" : undefined}
                            role={props.onOpen ? "link" : undefined}
                            aria-label={`${u.filename}, ${formatPeriod(u.first_date, u.last_date)}`}
                            onClick={open}
                            onKeyDown={(e) => {
                              if (e.key === "Enter" || e.key === " ") {
                                e.preventDefault();
                                open();
                              }
                            }}
                            onMouseMove={(e) => showTip(e, body)}
                            onFocus={(e) => showTip(e, body)}
                            onBlur={hideTip}
                          >
                            <mask id={id()} maskUnits="userSpaceOnUse">
                              <rect x={r().x} y={y()} width={Math.max(0, r().w - 6)} height={ROW_H} fill={`url(#${uid}-fade)`} />
                            </mask>
                            {/* Hit area: the block plus its gap, never smaller than MIN_HIT. */}
                            <rect x={box(b, MIN_HIT).x} y={y() - 2} width={box(b, MIN_HIT).w} height={ROW_H + 4} fill="transparent" />
                            {/* Translucent fill, soft 1px edge (drawn inside, so neighbours keep their 2px gap). */}
                            <rect
                              x={r().x + 0.5}
                              y={y() + 0.5}
                              width={Math.max(1, r().w - 3)}
                              height={ROW_H - 1}
                              fill="var(--viz-saved)"
                              fill-opacity="0.3"
                              stroke="var(--viz-saved)"
                              stroke-opacity="0.5"
                              stroke-width="1"
                              class="transition-[fill-opacity] group-hover:[fill-opacity:0.5] group-focus-visible:[stroke:var(--ring)] group-focus-visible:[stroke-opacity:1] group-focus-visible:[stroke-width:2]"
                            />
                            <Show when={r().w > 48}>
                              <text
                                x={r().x + 6}
                                y={y() + ROW_H / 2}
                                dominant-baseline="central"
                                mask={`url(#${id()})`}
                                class="pointer-events-none fill-muted-foreground text-[10px]"
                              >
                                {u.filename}
                              </text>
                            </Show>
                          </g>
                        );
                      }}
                    </For>
                  )}
                </For>

                {/* Date axis: month starts, plus the first and last day at the ends. */}
                <g class="fill-muted-foreground text-[10px]">
                  <line x1={LABEL_W} x2={width() - RIGHT} y1={plotBottom() + 4} y2={plotBottom() + 4} class="stroke-border" />
                  <For each={ticks()}>
                    {(t) => (
                      <text x={t.x} y={plotBottom() + 16} text-anchor="middle">
                        {t.label}
                      </text>
                    )}
                  </For>
                </g>
              </svg>
            );
          }}
        </Show>

        {/* One tooltip for every mark. Text goes in as text, never as HTML. */}
        <Show when={tip()}>
          {(t) => (
            <div
              role="tooltip"
              class="pointer-events-none absolute z-30 w-max max-w-72 -translate-x-1/2 rounded-md border border-border bg-background px-2.5 py-1.5 text-xs text-foreground shadow-lg"
              style={{
                left: `${Math.min(Math.max(t().x, 120), Math.max(120, width() - 120))}px`,
                // Above the pointer, unless that would leave the chart: then below it.
                top: t().y > 90 ? `${t().y - 10}px` : `${t().y + 18}px`,
                transform: t().y > 90 ? "translate(-50%, -100%)" : "translate(-50%, 0)",
              }}
            >
              {t().body}
            </div>
          )}
        </Show>
      </div>

      {/* Key, then the same facts in words (so nothing depends on hovering). */}
      <Show when={data()}>
        {(d) => (
          <div class="flex flex-col gap-1.5 text-xs text-muted-foreground">
            <div class="flex flex-wrap items-center gap-x-4 gap-y-1">
              <Key swatch={<rect x="0.5" y="0.5" width="13" height="9" fill="var(--viz-saved)" fill-opacity="0.3" stroke="var(--viz-saved)" stroke-opacity="0.5" />}>Saved statement</Key>
              <Key
                swatch={
                  <>
                    <defs>
                      <pattern id={`${uid}-key`} width="4" height="4" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
                        <line x1="0" y1="0" x2="0" y2="4" stroke="var(--viz-gap)" stroke-width="1.5" />
                      </pattern>
                    </defs>
                    <rect width="14" height="10" fill={`url(#${uid}-key)`} />
                  </>
                }
              >
                Gap between statements
              </Key>
              <Show when={d().pending}>
                <Key
                  swatch={
                    <rect x="1" y="1" width="12" height="8" rx="2" fill="var(--viz-pending)" fill-opacity="0.25" stroke="var(--viz-pending)" stroke-dasharray="3 2" />
                  }
                >
                  This statement
                </Key>
              </Show>
            </div>
            <Show when={d().effect}>
              {(e) => (
                <p class="text-foreground">
                  {e().first
                    ? "The first statement in this account."
                    : [
                        e().fills > 0 && `Fills ${dayCount(e().fills)} of gaps`,
                        e().extends > 0 && `adds ${dayCount(e().extends)} before or after what's saved`,
                        e().overlap > 0 && `overlaps ${dayCount(e().overlap)} already covered (those rows are updated, not duplicated)`,
                      ]
                        .filter(Boolean)
                        .join("; ")
                        .replace(/^./, (c) => c.toUpperCase()) + "."}
                </p>
              )}
            </Show>
            <Show when={d().gaps.length > 0} fallback={<p>No gaps between statements{d().pending ? " once this one is saved" : ""}.</p>}>
              <p>
                <span class="font-medium text-foreground">
                  {d().gaps.length} {d().gaps.length === 1 ? "gap" : "gaps"} between statements{d().pending ? " once this one is saved" : ""}:
                </span>{" "}
                {d()
                  .gaps.map((g) => `${spanText(g)} (${dayCount(days(g))})`)
                  .join(", ")}
              </p>
            </Show>
            <Show when={d().undated > 0}>
              <p>
                {d().undated} {d().undated === 1 ? "statement has" : "statements have"} no dated rows, so{" "}
                {d().undated === 1 ? "it isn't" : "they aren't"} shown.
              </p>
            </Show>
          </div>
        )}
      </Show>
    </div>
  );
}

function Key(props: { swatch: JSX.Element; children: JSX.Element }) {
  return (
    <span class="flex items-center gap-1.5">
      <svg width="14" height="10" aria-hidden="true">
        {props.swatch}
      </svg>
      {props.children}
    </span>
  );
}

/** "Overlaps x.csv by 19 days" lines for a tooltip (rows in both are stored once). */
function Overlaps(props: { list: { name: string; days: number }[] }) {
  return (
    <Show when={props.list.length > 0}>
      <div class="mt-1">
        <For each={props.list}>
          {(o) => (
            <p>
              Overlaps {o.name} by {dayCount(o.days)}
            </p>
          )}
        </For>
      </div>
    </Show>
  );
}
