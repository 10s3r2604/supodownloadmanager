// Tiny SVG charts for speeds.

function points(values: number[], width: number, height: number, max: number, pad = 2) {
  const n = Math.max(values.length, 2);
  return values.map((v, i) => {
    const x = (i / (n - 1)) * width;
    const y = pad + (1 - Math.min(1, v / max)) * (height - pad * 2);
    return `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`;
  });
}

/** A rounded upper bound for a chart axis: 1, 2, 5, 10, 20, 50 … MB/s. */
export function niceMax(value: number): number {
  const mb = 1024 * 1024;
  const target = Math.max(1, (value * 1.15) / mb);
  const exp = 10 ** Math.floor(Math.log10(target));
  for (const step of [1, 2, 5, 10]) {
    if (step * exp >= target) return step * exp * mb;
  }
  return 10 * exp * mb;
}

export function Sparkline(props: { values: number[]; width?: number; height?: number; className?: string }) {
  const { values, width = 64, height = 20 } = props;
  if (values.length < 2) return <svg width={width} height={height} className={props.className} />;
  const max = Math.max(...values, 1) * 1.1;
  return (
    <svg width={width} height={height} className={props.className} aria-hidden="true">
      <path d={points(values, width, height, max).join(" ")} fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" />
    </svg>
  );
}

/** An area chart that fills its container; `slots` fixes the time window. */
export function AreaChart(props: { values: number[]; max: number; slots: number; id: string }) {
  const width = 800;
  const height = 130;
  const pad = 6;
  // The newest sample sits at the right edge; older ones scroll left until they drop off.
  const values = props.values.slice(-props.slots);
  const step = width / (props.slots - 1);
  const x = (i: number) => width - (values.length - 1 - i) * step;
  const y = (v: number) => pad + (1 - Math.min(1, v / props.max)) * (height - pad * 2);
  const line = values.map((v, i) => `${i ? "L" : "M"}${x(i).toFixed(1)} ${y(v).toFixed(1)}`).join(" ");
  return (
    <svg viewBox={`0 0 ${width} ${height}`} preserveAspectRatio="none" className="area-chart" aria-hidden="true">
      <defs>
        <linearGradient id={props.id} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="currentColor" stopOpacity="0.3" />
          <stop offset="1" stopColor="currentColor" stopOpacity="0" />
        </linearGradient>
      </defs>
      {[0.2, 0.4, 0.6, 0.8].map((f) => (
        <line key={f} x1="0" x2={width} y1={height * f} y2={height * f} className="grid-line" />
      ))}
      {values.length > 1 && (
        <>
          <path d={`${line} L${width} ${height} L${x(0).toFixed(1)} ${height} Z`} fill={`url(#${props.id})`} />
          <path d={line} fill="none" stroke="currentColor" strokeWidth="2" className="glow-line" vectorEffect="non-scaling-stroke" />
        </>
      )}
    </svg>
  );
}

export function SegmentBlocks(props: { fills: number[]; className?: string }) {
  return (
    <div className={`segment-blocks ${props.className ?? ""}`}>
      {props.fills.map((fill, i) => (
        <i key={i}>
          <b style={{ width: `${fill * 100}%` }} />
        </i>
      ))}
    </div>
  );
}
