import { marked } from "marked";
import sanitizeHtml from "sanitize-html";

export function renderMarkdown(value: string): string {
	return sanitizeHtml(marked.parse(value, { async: false, breaks: true }), {
		allowedTags: sanitizeHtml.defaults.allowedTags.filter(
			(tag) => tag !== "img",
		),
	});
}
