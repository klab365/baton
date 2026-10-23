<script lang="ts">
import DetailPanel from "../../components/DetailPanel.svelte";
import PageHeader from "../../components/PageHeader.svelte";
let { data } = $props();
</script>
<svelte:head><title>Settings · Baton</title></svelte:head>
<PageHeader eyebrow="Settings" title="Project" description="Project details are read from the current Baton configuration." />
<DetailPanel title="Project details">
	<dl class="details"><dt>Name</dt><dd>{data.project.name}</dd><dt>Directory</dt><dd><code>{data.project.directory}</code></dd><dt>Configuration version</dt><dd>{data.project.status === "ready" ? data.project.config.schemaVersion : "–"}</dd></dl>
</DetailPanel>
<p class="muted">The project name is always derived from its directory name.</p>
