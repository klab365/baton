import { Type } from "typebox";
import { writeFileSync } from "node:fs";

export default function (pi) {
  const allowed = JSON.parse(process.env.BATON_PI_ALLOWED_OUTCOMES);
  let reported = false;
  pi.registerTool({
    name: "report_outcome",
    label: "Report outcome",
    description: `Report the pipeline decision and handoff. Allowed outcomes: ${allowed.join(", ")}.`,
    parameters: Type.Object({ outcome: Type.String(), summary: Type.String() }),
    async execute(_id, params) {
      if (reported) throw new Error("An outcome was already reported");
      if (!allowed.includes(params.outcome)) throw new Error("Outcome is not allowed for this step");
      writeFileSync(process.env.BATON_PI_OUTCOME_FILE, JSON.stringify(params), { flag: "wx", mode: 0o600 });
      reported = true;
      return { content: [{ type: "text", text: "Outcome recorded." }], details: {} };
    },
  });
}
