import { Type } from "typebox";
import { writeFileSync } from "node:fs";

export default function (pi) {
  const allowed = JSON.parse(process.env.BATON_PI_ALLOWED_OUTCOMES);
  let reported = false;
  pi.registerTool({
    name: "report_outcome",
    label: "Report outcome",
    description: `Report the pipeline decision and complete handoff. For blocked, include blocker_reason and human_action. Allowed outcomes: ${allowed.join(", ")}.`,
    parameters: Type.Object({ outcome: Type.String(), summary: Type.String(), blocker_reason: Type.Optional(Type.String()), human_action: Type.Optional(Type.String()) }),
    async execute(_id, params) {
      if (reported) throw new Error("An outcome was already reported");
      if (!allowed.includes(params.outcome)) throw new Error("Outcome is not allowed for this step");
      if (typeof params.summary !== "string" || !params.summary.trim()) throw new Error("Summary must not be empty");
      if (params.outcome === "blocked") {
        if (!params.blocker_reason?.trim() || !params.human_action?.trim()) throw new Error("Blocked outcomes require a blocker reason and a specific human action");
        params.summary += `\n\nBlocker: ${params.blocker_reason.trim()}\nHuman action needed: ${params.human_action.trim()}`;
      }
      if (Buffer.byteLength(params.summary) > 16384) throw new Error("Summary must contain 1 to 16384 bytes");
      writeFileSync(process.env.BATON_PI_OUTCOME_FILE, JSON.stringify(params), { flag: "wx", mode: 0o600 });
      reported = true;
      return { content: [{ type: "text", text: "Outcome recorded." }], details: {} };
    },
  });
}
