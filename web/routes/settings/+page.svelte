<script lang="ts">let { data } = $props();
</script>
<svelte:head><title>Settings · Baton</title></svelte:head>
<p class="eyebrow">Settings</p><h1>Project</h1>
<dl class="details"><dt>Name</dt><dd>{data.project.name}</dd><dt>Directory</dt><dd><code>{data.project.directory}</code></dd><dt>Configuration version</dt><dd>{data.project.status === "ready" ? data.project.config.schemaVersion : "–"}</dd></dl>
<p class="muted">The project name is always derived from its directory name.</p>
