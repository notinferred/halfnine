/** One node of Postgres's JSON plan; only the fields shown here are named. */
type PlanNode = {
  "Node Type": string;
  "Relation Name"?: string;
  "Index Name"?: string;
  "CTE Name"?: string;
  Alias?: string;
  "Startup Cost"?: number;
  "Total Cost"?: number;
  "Plan Rows"?: number;
  "Actual Total Time"?: number;
  "Actual Rows"?: number;
  "Actual Loops"?: number;
  Filter?: string;
  "Index Cond"?: string;
  "Hash Cond"?: string;
  "Merge Cond"?: string;
  "Join Filter"?: string;
  "Rows Removed by Filter"?: number;
  Plans?: PlanNode[];
};

type Explained = { Plan: PlanNode; "Planning Time"?: number; "Execution Time"?: number };

const CONDITIONS = ["Index Cond", "Hash Cond", "Merge Cond", "Join Filter", "Filter"] as const;

/** The plan as a tree, each step with its estimate and, after Analyze, what it really took. */
export function PlanView(props: { plan: string; analyzed: boolean }) {
  let explained: Explained | undefined;
  try {
    explained = (JSON.parse(props.plan) as Explained[])[0];
  } catch {
    explained = undefined;
  }
  if (!explained) return <pre className="plan-raw">{props.plan}</pre>;
  const total = explained.Plan["Actual Total Time"];

  return (
    <div className="plan">
      <p className="plan-summary muted">
        {props.analyzed
          ? `Ran in ${explained["Execution Time"]?.toFixed(1)} ms, planned in ${explained["Planning Time"]?.toFixed(1)} ms, then rolled back.`
          : "Estimated by the planner, without running the statement."}
      </p>
      <PlanStep node={explained.Plan} total={total} />
    </div>
  );
}

function PlanStep({ node, total }: { node: PlanNode; total: number | undefined }) {
  const target = node["Relation Name"] ?? node["CTE Name"];
  const time = node["Actual Total Time"];
  const share = total && time !== undefined ? Math.min(1, time / total) : null;

  return (
    <div className="plan-step">
      <div className="plan-line">
        <span className="plan-name">{node["Node Type"]}</span>
        {target && (
          <span>
            on <span className="mono">{target}</span>
            {node.Alias && node.Alias !== target && <span className="muted"> as {node.Alias}</span>}
          </span>
        )}
        {node["Index Name"] && (
          <span className="muted">
            using <span className="mono">{node["Index Name"]}</span>
          </span>
        )}
        <span className="plan-numbers muted">
          {time !== undefined
            ? `${time.toFixed(2)} ms · ${node["Actual Rows"]?.toLocaleString()} rows${(node["Actual Loops"] ?? 1) > 1 ? ` × ${node["Actual Loops"]}` : ""}`
            : `cost ${node["Total Cost"]?.toFixed(0)} · ${node["Plan Rows"]?.toLocaleString()} rows`}
        </span>
      </div>
      {share !== null && (
        <div className="plan-bar" aria-hidden="true">
          <span className="plan-fill" style={{ width: `${share * 100}%` }} />
        </div>
      )}
      {CONDITIONS.filter((c) => node[c]).map((c) => (
        <div key={c} className="plan-condition mono muted">
          {c}: {node[c]}
          {c === "Filter" && node["Rows Removed by Filter"] ? ` (removed ${node["Rows Removed by Filter"].toLocaleString()} rows)` : ""}
        </div>
      ))}
      {node.Plans?.map((child, i) => (
        <PlanStep key={i} node={child} total={total} />
      ))}
    </div>
  );
}
