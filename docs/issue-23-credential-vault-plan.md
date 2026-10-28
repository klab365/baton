# Plan: Lokalen Credential Vault einführen (#23)

> Architekturentscheidung: Runtime-Profile wurden verworfen. Provider, Modell und `credentialId` liegen direkt am Agent; Schema v6 migriert diese Werte aus Profilen in Schema v7. Die untenstehende ursprüngliche Schrittplanung dokumentiert den Startpunkt und ist in diesem Punkt überholt.

## Goal
Wiederverwendbare lokale Credentials sicher verwalten und Rig/OpenAI-/ChatGPT-Profile über Credential-IDs statt über Secret-Daten konfigurieren.

## Approach
Credential-Metadaten und Secrets werden klar getrennt: Das Core definiert Modelle, Use-Cases und Ports; `baton-adapters` implementiert den lokalen Secret Store; Server und UI exponieren ausschließlich redigierte Metadaten. Runtime-Profile bleiben versionierte Projektkonfiguration, enthalten aber nur Credential-Referenzen.

> Hinweis: PR #20 ist im aktuellen Stand durch die Runtime-Profile-Überarbeitung nicht mehr enthalten. Die Migration muss daher sowohl vorhandene historische lokale Provider-Daten berücksichtigen als auch die aktuelle Runtime-Profile-Struktur erweitern.

## Steps

1. **Credential-Domain und Secret-Store-Port definieren**
   - Files: `crates/baton-core/src/domain/credentials.rs` (neu), `crates/baton-core/src/ports/credentials.rs` (neu), `crates/baton-core/src/domain/mod.rs`, `crates/baton-core/src/ports/mod.rs`, `crates/baton-core/src/lib.rs`
   - Credential-Modell mit lokaler stabiler ID, Name, Typ, Erstell-/Änderzeit und nicht-sensitivem Status.
   - Zulässige Typen fest definieren: `openai_api_key`, `chatgpt_session`, `github_pat`, `github_copilot_session`, `azure_devops_pat`, `sonar_token`.
   - Port-Schnittstelle für Metadaten-CRUD, Secret setzen/lesen/löschen sowie status-/testfähige Credentials definieren. Secrets dürfen nie Bestandteil von Domain-DTOs sein.

2. **Runtime-Profile um Credential-Referenzen erweitern und Config migrieren**
   - Files: `crates/baton-core/src/domain/runtime_profiles.rs`, `crates/baton-core/src/domain/project.rs`, `crates/baton-core/tests/{project_config,runtime_profiles}.rs`, `crates/baton-core/tests/fixture_config.json`
   - Rig-Profile erhalten eine optionale bzw. für authentifizierte Provider erforderliche `credentialId`-Referenz.
   - Validierung ergänzt Referenzformat, Provider-zu-Credential-Typ-Kompatibilität und verständliche Fehler für fehlende Credentials.
   - Schema-Version erhöhen und bestehende v6-Konfigurationen ohne Secret-Werte verlustfrei migrieren.
   - Sicherstellen, dass Config-Snapshots und Run-Daten ausschließlich IDs enthalten.

3. **Lokalen, restriktiven Vault-Adapter implementieren**
   - Files: `crates/baton-adapters/src/credentials/*` (neu), `crates/baton-adapters/src/lib.rs`, `crates/baton-adapters/Cargo.toml`, ggf. `docs/local-credential-storage.md` (neu)
   - OS-Keychain bevorzugen; falls erforderlich einen dokumentierten lokalen Store mit restriktiven Berechtigungen und atomaren Writes verwenden.
   - Metadaten getrennt von Secret-Werten speichern; keine Credentials unter `.baton/config.json` oder in versionierten Dateien ablegen.
   - Fehler des Secret Stores in sichere, handlungsfähige Fehlermeldungen übersetzen, ohne Secret-Inhalte oder Backend-Diagnostik auszugeben.
   - Historische lokale Provider-Daten aus PR #20 erkennen und kontrolliert in Credentials und Profil-Referenzen migrieren; Migration muss idempotent und fehlertolerant sein.

4. **Credential-Use-Cases und Referenzschutz implementieren**
   - Files: `crates/baton-core/src/features/credentials.rs` (neu), `crates/baton-core/src/features/mod.rs`, `crates/baton-core/src/features/runtime_profiles.rs`
   - Medi-Kommandos für Auflisten, Inspizieren, Anlegen, Metadaten ändern, Secret ersetzen, Testen und Löschen erstellen.
   - Beim Löschen alle lokalen Referenzen prüfen; bei Nutzung durch Runtime-Profile (später Capabilities) mit konkretem Hinweis blockieren.
   - Credential-Test als typabhängige Operation modellieren; nicht unterstützte Typen klar ausweisen.

5. **Credentials in Runtime und Server-Komposition injizieren**
   - Files: `crates/baton-adapters/src/runtime/{profiles.rs,rig/mod.rs}`, `crates/baton-server/src/{lib.rs,mediator.rs}`
   - `AgentRuntimeFactory` bzw. Rig-Adapter erhält Zugriff auf den Credential-Port, löst die ID erst zur Laufzeit auf und reicht Secrets nur an den benötigten Provider-Client weiter.
   - OpenAI und ChatGPT separat behandeln; ein ChatGPT-Session-Credential darf nicht als OpenAI-API-Key verwendet werden.
   - Fehlende, ungültige oder nicht verfügbare Credentials müssen Runs sicher und ohne Secret-Leak fehlschlagen lassen.

6. **Redigierte Credential-API bereitstellen**
   - Files: `crates/baton-server/src/routes/credentials.rs` (neu), `crates/baton-server/src/routes/mod.rs`, `crates/baton-server/src/api/dto.rs`, `crates/baton-server/src/generate_types.rs`
   - Endpunkte für Liste, Detail-Metadaten, Create, Secret-Replacement, Test und Delete erstellen.
   - Request-DTOs mit Secret-Feldern nur für Schreiboperationen verwenden; Response-DTOs enthalten niemals Secret, Session, Device-Code oder Token.
   - API- und Logging-Pfade gezielt auf Redaction prüfen.

7. **Settings-Oberfläche und Proxy ergänzen**
   - Files: `web/routes/settings/+page.svelte`, `web/routes/api/credentials/[...path]/+server.ts` (neu), generierte Typen unter `web/lib/generated/`
   - Credential-Liste mit Typ, Name, Status und „zuletzt geändert“ anzeigen.
   - Flows für Anlegen, Secret ersetzen, Test, Löschen mit Bestätigung und nutzungsblockierte Löschung implementieren.
   - Runtime-Profil-Editor um Credential-Auswahl nach kompatiblem Provider/Typ erweitern; eingegebene Secrets nach Submit sofort aus dem Client-State entfernen.

8. **Sicherheits-, Migrations- und Fehlerpfade testen**
   - Files: neue/erweiterte Tests in `crates/baton-core/tests/`, `crates/baton-adapters/tests/`, `crates/baton-server/tests/`; ggf. UI-Tests, falls vorhanden
   - Abdecken: API-/Log-/Snapshot-Redaction, fehlende Secrets, Store-Fehler und atomare Konsistenz, Referenzschutz, v6-/historische-Provider-Migration, gemeinschaftliche Referenz eines Credentials sowie OpenAI-/ChatGPT-Laufzeitfehler.
   - Validierung: `mise run test`, `mise run lint-rust`, `mise run check`.

## Dependencies between steps
- Schritte 1–3 bilden die Grundlage für Features, Runtime und API.
- Schritt 2 muss vor Referenzschutz und UI-Auswahl erfolgen.
- Schritt 5 benötigt die fertigen Ports und den Vault-Adapter.
- Schritt 8 begleitet alle Schritte, insbesondere Migration und Persistenz.

## Out of scope
- Credential-Verwendung durch noch nicht existierende Capabilities oder Agent Tools.
- Synchronisation, Sharing oder Export von Credentials.
- Automatisches Übernehmen von Secrets in Git, Backups oder Run-Historien.

## Open questions
- Soll der Fallback-Store nur permission-restricted oder zusätzlich verschlüsselt sein, falls kein OS-Keychain verfügbar ist?
- Welche konkreten Provider-Tests sind für ChatGPT-Session und GitHub-Copilot technisch zuverlässig verfügbar?

## Estimated complexity
**Large** — betrifft Domain-Grenzen, sichere Persistenz, Schema-/Alt-Daten-Migration, Runtime-Injektion, HTTP-API und Settings-UI.
