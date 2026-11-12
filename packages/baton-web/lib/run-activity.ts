export type ActivityItem = { label: string; content: string };

// Transcripts are runtime-specific; unknown shapes remain available in raw diagnostics.
export function parseRunActivity(content: string): ActivityItem[] {
	try {
		const transcript: unknown = JSON.parse(content);
		if (
			transcript &&
			typeof transcript === "object" &&
			!Array.isArray(transcript)
		) {
			const event = transcript as Record<string, unknown>;
			const name = typeof event.tool === "string" ? event.tool : "unknown";
			switch (event.kind) {
				case "modelRequest":
					return [
						{
							label: "Model request",
							content: `Messages: ${Array.isArray(event.messageRoles) ? event.messageRoles.join(", ") : "unavailable"}; configured tools: ${Array.isArray(event.toolNames) ? event.toolNames.join(", ") || "none" : "unavailable"}${event.messageCountTruncated || event.toolListTruncated || event.toolNameTruncated ? " · metadata truncated" : ""}`,
						},
					];
				case "modelResponse":
					return [
						{
							label: `Model response: ${typeof event.status === "string" ? event.status : "status unavailable"}`,
							content: "Response text is not recorded in the activity trace.",
						},
					];
				case "wireRequest":
					return [
						{
							label: "Provider request",
							content: `Message roles: ${Array.isArray(event.roles) ? event.roles.join(", ") || "none" : "unavailable"}${event.messageCountTruncated ? " · roles truncated" : ""}`,
						},
					];
				case "toolCall":
					return [
						{
							label: `Tool call: ${name}`,
							content: "Arguments are omitted from the persisted trace.",
						},
					];
				case "toolResult":
					return [
						{
							label: `Tool result: ${name} · ${typeof event.status === "string" ? event.status : "status unavailable"}`,
							content: "Result contents are omitted from the persisted trace.",
						},
					];
				case "traceTruncated":
					return [
						{
							label: "Trace capture truncated",
							content:
								typeof event.content === "string"
									? event.content
									: "Some activity was omitted at capture limits.",
						},
					];
			}
		}
		const messages = Array.isArray(transcript)
			? transcript
			: (transcript as { messages?: unknown[] })?.messages;
		if (!Array.isArray(messages)) return [];
		const items: ActivityItem[] = [];
		for (const message of messages) {
			if (!message || !Array.isArray(message.content)) continue;
			for (const part of message.content) {
				if (part?.type === "toolcall") {
					items.push({
						label: `Tool call: ${part.function?.name ?? "unknown"}`,
						content: JSON.stringify(part.function?.arguments ?? {}, null, 2),
					});
				} else if (part?.type === "toolresult") {
					items.push({
						label: `Tool result: ${part.name ?? "unknown"}`,
						content: JSON.stringify(part.content ?? [], null, 2),
					});
				} else if (
					message.role === "assistant" &&
					part?.type === "text" &&
					typeof part.text === "string" &&
					part.text.trim()
				) {
					items.push({ label: "Agent response", content: part.text });
				}
			}
		}
		return items;
	} catch {
		return [];
	}
}
