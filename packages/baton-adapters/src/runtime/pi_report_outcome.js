import { Type } from "typebox";
import { readFileSync, writeFileSync } from "node:fs";

const allowedFields = new Set(["version", "summary", "findings", "verification", "nextAction"]);
const findingFields = new Set(["id", "status", "description"]);
const evidenceFields = new Set(["description", "result"]);
const statuses = new Set(["open", "resolved", "accepted"]);

function validateHandoff(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("structured_handoff must be an object");
  for (const key of Object.keys(value)) if (!allowedFields.has(key)) throw new Error(`structured_handoff has unknown field: ${key}`);
  if (value.version !== 1) throw new Error("structured_handoff.version must be 1");
  if (typeof value.summary !== "string" || !value.summary.trim()) throw new Error("structured_handoff.summary must not be blank");
  const findings = value.findings === undefined ? [] : value.findings;
  if (!Array.isArray(findings) || findings.length > 128) throw new Error("structured_handoff.findings must be an array of at most 128 items");
  const ids = new Set();
  for (const finding of findings) {
    if (!finding || typeof finding !== "object" || Array.isArray(finding)) throw new Error("Each finding must be an object");
    for (const key of Object.keys(finding)) if (!findingFields.has(key)) throw new Error(`Finding has unknown field: ${key}`);
    if (typeof finding.id !== "string" || !finding.id.trim() || ids.has(finding.id)) throw new Error("Finding IDs must be nonblank and unique");
    ids.add(finding.id);
    if (!statuses.has(finding.status)) throw new Error(`Finding ${finding.id} status must be open, resolved, or accepted`);
    if (typeof finding.description !== "string" || !finding.description.trim()) throw new Error(`Finding ${finding.id} description must not be blank`);
  }
  const verification = value.verification === undefined ? [] : value.verification;
  if (!Array.isArray(verification)) throw new Error("structured_handoff.verification must be an array");
  for (const evidence of verification) {
    if (!evidence || typeof evidence !== "object" || Array.isArray(evidence)) throw new Error("Each verification item must be an object");
    for (const key of Object.keys(evidence)) if (!evidenceFields.has(key)) throw new Error(`Verification evidence has unknown field: ${key}`);
    if (typeof evidence.description !== "string" || !evidence.description.trim() || typeof evidence.result !== "string" || !evidence.result.trim()) throw new Error("Verification evidence description and result must not be blank");
  }
  if (value.nextAction !== undefined && (typeof value.nextAction !== "string" || !value.nextAction.trim())) throw new Error("structured_handoff.nextAction must not be blank when provided");
  const normalized = { version: value.version, summary: value.summary, findings, verification };
  if (value.nextAction !== undefined) normalized.nextAction = value.nextAction;
  if (Buffer.byteLength(JSON.stringify(normalized)) > 16384) throw new Error("structured_handoff exceeds 16384 bytes");
}

export default function (pi) {
  const allowed = JSON.parse(process.env.BATON_PI_ALLOWED_OUTCOMES);
  let reported = false;
  pi.registerTool({
    name: "get_structured_handoff",
    label: "Retrieve structured handoff",
    description: "Retrieve a complete persisted structured report by producing attempt number, as listed in the bounded-context notice.",
    parameters: Type.Object({ attempt: Type.Number() }),
    async execute(_id, params) {
      if (!Number.isInteger(params.attempt) || params.attempt < 1) throw new Error("attempt must be a positive integer");
      const history = JSON.parse(readFileSync(process.env.BATON_PI_HANDOFF_HISTORY_PATH, "utf8"));
      const report = history.find(entry => entry.attempt === params.attempt);
      if (!report) throw new Error("No structured handoff exists for that attempt");
      return { content: [{ type: "text", text: JSON.stringify(report) }], details: {} };
    },
  });
  if (process.env.BATON_PI_REPORT_OUTCOME_ENABLED !== "false") pi.registerTool({
    name: "report_outcome",
    label: "Report outcome",
    description: `Report the pipeline decision and complete handoff. For blocked, include blocker_reason and an optional human_action. Allowed outcomes: ${allowed.join(", ")}. Pipeline agent steps require structured_handoff; non-pipeline agent calls may omit it. When supplied, it must be a version-1 object: {"version":1,"summary":"...","findings":[{"id":"F-1","status":"open","description":"..."}],"verification":[{"description":"command/check","result":"agent-reported result"}],"nextAction":"..."}. Findings and verification are optional arrays. Status is open, resolved, or accepted; IDs must be stable and unique. Agent verification is an assertion, not independently verified. Invalid reports can be corrected and resubmitted.`,
    parameters: Type.Object({ outcome: Type.String(), summary: Type.String(), blocker_reason: Type.Optional(Type.String()), human_action: Type.Optional(Type.String()), structured_handoff: Type.Optional(Type.Any()) }),
    async execute(_id, params) {
      if (reported) throw new Error("An outcome was already reported");
      if (!allowed.includes(params.outcome)) throw new Error("Outcome is not allowed for this step");
      if (typeof params.summary !== "string" || !params.summary.trim()) throw new Error("Summary must not be empty");
      if (params.outcome === "blocked") {
        if (!params.blocker_reason?.trim()) throw new Error("Blocked outcomes require a blocker reason");
        params.summary += `\n\nBlocker: ${params.blocker_reason.trim()}`;
        if (params.human_action?.trim()) params.summary += `\nHuman action needed: ${params.human_action.trim()}`;
      }
      if (Buffer.byteLength(params.summary) > 16384) throw new Error("Summary must contain 1 to 16384 bytes");
      if (params.structured_handoff != null) validateHandoff(params.structured_handoff);
      writeFileSync(process.env.BATON_PI_OUTCOME_FILE, JSON.stringify(params), { flag: "wx", mode: 0o600 });
      reported = true;
      return { content: [{ type: "text", text: "Outcome recorded." }], details: {} };
    },
  });
}
