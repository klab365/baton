# 001 – Scaffolding und erste Weboberfläche

## Ziel

Baton erhält ein lauffähiges Bun-/SvelteKit-/TypeScript-Grundgerüst und eine erste Weboberfläche. Ein Nutzer kann Baton für ein Projekt starten, eine minimale Projektkonfiguration anlegen und anschließend eine übersichtliche Projektoberfläche öffnen.

Dieses Feature schafft die Grundlage für Baton. Es implementiert noch keine Agenten-Orchestrierung.

## Ausgangslage

- Das Repository enthält ein minimales Bun-Projekt mit `package.json`, `index.ts`, `tsconfig.json` und `bun.lock`.
- SvelteKit, UI-Komponenten und Tests sind noch nicht vorhanden.
- `Readme.md` beschreibt die Konzepte Control Plane und Data Planes.
- Es gibt noch keine bestehenden Implementierungsmuster oder Plan-Dokumente.

## Ansatz

Baton ist eine einzelne SvelteKit-Anwendung mit CLI und Weboberfläche. Sie bündelt Verwaltung und spätere Ausführung; es gibt keine getrennten Anwendungen für Control Plane und Data Plane(s). Serverseitige Module lesen und schreiben die Projektkonfiguration; die Oberfläche greift ausschließlich über SvelteKit-Load-Funktionen und Form Actions darauf zu.

Kein Monorepo und kein separater API-Server. Eine klar abgegrenzte CLI startet den lokalen Server mit einem expliziten Projektpfad. Ausführungsorte bleiben ein Konfigurationskonzept innerhalb derselben Baton-Anwendung und werden in diesem Feature noch nicht implementiert.

### Technische Leitplanken

- Bun als Runtime und Paketmanager, SvelteKit mit TypeScript und strikter Typprüfung.
- Bun-kompatiblen SvelteKit-Adapter mit Produktions-Build vor der weiteren Umsetzung prüfen und Versionen festhalten.
- Keine Annahme treffen, dass der Vite-Entwicklungsserver bereits die spätere CLI-Laufzeit darstellt.
- Flue SDK erst bei der tatsächlichen Agentenintegration einbinden. Paket, Version und API sind vorher zu verifizieren; dieses Feature benötigt das SDK noch nicht.
- Projektdateien bleiben serverseitig; Browser erhalten keine beliebigen Dateisystemzugriffe.
- Zunächst eine lokale Einzelbenutzer-Anwendung ohne Authentifizierung; der Server bindet ausschließlich an Loopback.

## Funktionsumfang der ersten Version

### 1. Lokaler Start

```bash
# Entwicklung der Baton-Oberfläche
bun run dev

# Nach Build und lokaler Verknüpfung des CLI-Befehls, im Zielprojekt
baton .
```

- Die CLI löst den Projektpfad absolut auf und prüft, ob es ein existierendes Verzeichnis ist.
- Der Projektpfad wird explizit an den Server übergeben; er ist nicht automatisch das Verzeichnis der Baton-Installation.
- Der Server startet lokal, zeigt seine URL an und öffnet den Browser. Falls dies nicht möglich ist, bleibt die URL manuell nutzbar.
- Ein belegter Port erzeugt eine verständliche Fehlermeldung. Automatische Portsuche ist zunächst nicht erforderlich.
- Abbruch beendet den gestarteten Server sauber.
- `bun run dev` nutzt ohne explizite Projektpfadangabe das aktuelle Arbeitsverzeichnis und dokumentiert dieses Verhalten.

### 2. Ersteinrichtung

Fehlt `.baton/config.json`, erscheint ein kompakter Einrichtungsdialog:
1. Projektname als schreibgeschützten Namen des Projektverzeichnisses anzeigen.
2. Projektpfad und Umfang der Einrichtung anzeigen.
3. Konfiguration bewusst über „Projekt einrichten“ speichern.
4. Zur Projektübersicht wechseln.

Der erste Start allein legt noch keine Dateien an. Der vollständige Assistent für Capabilities, Ausführungsorte, Agenten und Pipelines folgt später. Die Oberfläche weist darauf hin, dass diese Konfigurationen noch nicht verfügbar sind.

### 3. Projektoberfläche

- Gemeinsames Layout mit Baton-Branding, aus dem Projektverzeichnis abgeleitetem Projektnamen und Navigation.
- Navigation: Übersicht, Aufgaben, Pipelines, Agenten, Ausführungsorte und Einstellungen.
- Übersicht mit Einrichtungsstatus, kurzen Erläuterungen und Links zu den einzelnen Bereichen.
- Aufgaben, Pipelines, Agenten und Ausführungsorte erhalten informative Leerseiten: Zweck, aktueller Zustand und Hinweis auf spätere Funktionen.
- Einstellungen zeigen den nicht editierbaren Projektnamen, Projektpfad und Konfigurationsversion zunächst nur lesend an.
- Keine erfundenen laufenden Agenten, Erfolgsstatistiken oder scheinbar bedienbaren Start-Buttons.
- Responsive Darstellung für Desktop und schmale Bildschirme, semantische Formulare, sichtbarer Tastaturfokus und verständliche Fehlermeldungen.

### 4. Minimale Projektkonfiguration

Versionierte Datei `.baton/config.json`:

```json
{
	"schemaVersion": 1
}
```

- Nur das aktuell benötigte Schema implementieren; keine spekulativen Agenten- oder Pipeline-Schemas festschreiben.
- Projektname und Projektpfad nicht speichern: Der Projektname wird immer aus dem Verzeichnisnamen abgeleitet und die Konfiguration bleibt zwischen Rechnern übertragbar.
- Lokale Einstellungen, Zugangsdaten und Laufdaten gehören nicht in diese Datei. Sie werden in diesem Feature noch nicht angelegt.
- Bestehende gültige Konfiguration laden; beschädigte Dateien oder unbekannte Schema-Versionen als Fehler anzeigen und niemals automatisch überschreiben.
- Schreiben erst nach serverseitiger Validierung, über eine temporäre Datei im selben Verzeichnis und atomaren Austausch. Gleichzeitige Initialisierungsversuche dürfen eine bestehende Konfiguration nicht unbeabsichtigt ersetzen.

## Umsetzungsschritte

### 1. SvelteKit-Grundgerüst aufsetzen

- Bestehenden Bun-Stub durch die SvelteKit-Struktur ersetzen.
- Bun-kompatiblen Adapter auswählen und einen minimalen Produktionsstart verifizieren.
- TypeScript-Konfiguration auf SvelteKit umstellen; bestehende strikte Prüfungen soweit kompatibel beibehalten.
- Skripte für `dev`, `build`, `start`, `check` und `test` hinzufügen.
- Dateien: `package.json`, `bun.lock`, `tsconfig.json`, `svelte.config.js`, `vite.config.ts`, `src/app.html`, `src/app.d.ts`, `src/routes/+page.svelte`; bisherigen `index.ts` entfernen oder durch den dedizierten CLI-Einstieg ersetzen.
- Prüfung: Entwicklungsserver, Typprüfung, Build und Start des gebauten Servers funktionieren ohne UI-Funktionen.

### 2. CLI und Projektkontext implementieren

- Argumentverarbeitung, Projektpfadvalidierung, Serverstart und Browseröffnung implementieren.
- Einen zentralen serverseitigen Projektkontext bereitstellen; kein Projektpfad aus beliebigen Browserparametern.
- `baton` über den `bin`-Eintrag bereitstellen und den lokalen Build-/Link-Ablauf dokumentieren.
- Dateien: `src/cli.ts`, `src/lib/server/project-context.ts`, `package.json`, `Readme.md`.
- Prüfung: Start aus einem fremden Projektverzeichnis, Pfade mit Leerzeichen, ungültiger Pfad, belegter Port und sauberer Abbruch.

### 3. Konfigurationsspeicher ergänzen

- Typisiertes Schema und serverseitige Validierung anlegen.
- Zustände „nicht eingerichtet“, „bereit“ und „Konfigurationsfehler“ unterscheiden.
- Sicheres initiales Schreiben und erneutes Laden implementieren. Unbekannte Schema-Versionen abweisen; der Projektname wird aus dem validierten Projektpfad abgeleitet.
- Dateien: `src/lib/server/config/schema.ts`, `src/lib/server/config/store.ts`, `tests/config-store.test.ts`.
- Prüfung: temporäre Testverzeichnisse für fehlende, gültige und beschädigte Dateien sowie wiederholte und konkurrierende Initialisierung verwenden.

### 4. Einrichtung und gemeinsames Layout bauen

- Root-Load entscheidet anhand der Konfiguration zwischen Einrichtung, Projektübersicht und Fehleransicht.
- Initialisierung als serverseitige Form Action mit Validierung und Fehlerdarstellung umsetzen.
- Navigation und wiederverwendbare UI-Bausteine ergänzen; Gestaltung zunächst mit eigenem CSS und wenigen Komponenten statt einer umfangreichen Design-System-Abhängigkeit.
- Dateien: `src/routes/+layout.server.ts`, `src/routes/+layout.svelte`, `src/routes/+page.server.ts`, `src/routes/+page.svelte`, `src/lib/components/`, `src/app.css`.
- Prüfung: fehlende Konfiguration zeigt Einrichtung; gültige Einrichtung persistiert und bleibt nach Neuladen erhalten; beschädigte Konfiguration zeigt eine nicht destruktive Fehleransicht.

### 5. Erste Projektseiten ergänzen

- Übersicht, Leerseiten und lesende Einstellungen umsetzen.
- Alle Projektseiten vor Nutzung ohne gültige Konfiguration schützen; auch direkte URLs führen zur Einrichtung beziehungsweise Fehleransicht.
- Dateien: `src/routes/tasks/+page.svelte`, `src/routes/pipelines/+page.svelte`, `src/routes/agents/+page.svelte`, `src/routes/execution-targets/+page.svelte`, `src/routes/settings/+page.svelte`, gegebenenfalls zugehörige serverseitige Load-Dateien.
- Prüfung: Navigation und direkte Seitenaufrufe funktionieren; keine Aktionen suggerieren bereits vorhandene Orchestrierung.

### 6. Ende-zu-Ende absichern und dokumentieren

- Browser-Smoke-Tests für Ersteinrichtung, Wiederöffnen und Navigation ergänzen; isolierte temporäre Projektverzeichnisse verwenden.
- Einen Smoke-Test gegen den gebauten Server ausführen, nicht ausschließlich gegen Vite.
- Tastaturbedienung und schmale Bildschirmansicht prüfen.
- README um Installation, Entwicklung, Build, CLI-Verknüpfung, Projektstart und aktuelle Einschränkungen ergänzen; Konzept und bereits implementierte Funktionen klar trennen.
- Dateien: `tests/e2e/`, `playwright.config.ts`, `package.json`, `Readme.md`, gegebenenfalls `.gitignore` für generierte Build- und Testartefakte.
- Prüfung: `bun run check`, `bun test`, `bun run build` und Browser-Smoke-Tests erfolgreich.

## Abhängigkeiten zwischen den Schritten

- Schritt 1 muss Adapter und Produktionsstart klären, bevor die CLI darauf aufbaut.
- Konfigurationsspeicher benötigt den Projektkontext aus Schritt 2.
- Einrichtung und Projektseiten bauen auf dem Konfigurationsspeicher auf.
- Tests werden jeweils mit den Schritten ergänzt; Schritt 6 verbindet sie zu einem Ende-zu-Ende-Nachweis.

## Abnahmekriterien

- [ ] Das Repository besitzt ein funktionsfähiges Bun-/SvelteKit-/TypeScript-Grundgerüst.
- [ ] Der gebaute Server läuft unter Bun; der lokale CLI-Installationsweg ist reproduzierbar dokumentiert.
- [ ] `baton .` startet die Weboberfläche für das aufrufende Projekt, nicht für das Baton-Installationsverzeichnis.
- [ ] Ein Projekt ohne Konfiguration zeigt die Einrichtung und schreibt erst nach Bestätigung Dateien.
- [ ] Eine erfolgreiche Einrichtung erzeugt `.baton/config.json` mit Schema-Version; der Projektname entspricht immer dem Namen des Projektverzeichnisses.
- [ ] Ein erneuter Start lädt die bestehende Konfiguration ohne erneute Einrichtung.
- [ ] Ungültige Konfigurationen werden verständlich gemeldet und nicht überschrieben.
- [ ] Alle genannten Navigationsziele sind erreichbar und zeigen den tatsächlichen Implementierungsstand.
- [ ] Die Oberfläche ist mit Tastatur und auf schmalen Bildschirmen bedienbar.
- [ ] Typprüfung, Unit-Tests, Produktions-Build und Browser-Smoke-Tests sind erfolgreich.
- [ ] Es werden weder Zugangsdaten gespeichert noch externe Agenten oder Dienste aufgerufen.

## Nicht enthalten

- Tatsächliche Agenten- oder Pipeline-Ausführung und Flue-SDK-Anbindung.
- Vollständiger Einrichtungsassistent oder Editoren für Agenten, Capabilities und Pipelines.
- Anlegen und Persistieren von Aufgaben, Run-Historie, Streaming, Ergebnisse und Freigaben.
- GitHub-/Azure-DevOps-Integrationen und andere externe Capabilities.
- Remote-Ausführungsorte, Benutzerverwaltung, Authentifizierung und öffentliche Netzwerkfreigabe.
- Templates, Datenbank, Deployment-Plattform und Veröffentlichung eines Distributionspakets.

## Offene Entscheidungen und Risiken

- **Adapter und Bun-Laufzeit:** Die konkrete Adapterwahl ist eine technische Entscheidung in Schritt 1. Ein erfolgreicher Vite-Start allein genügt nicht als Kompatibilitätsnachweis.
- **CLI-Distribution:** Für dieses Feature genügt Build plus lokale Verknüpfung. Veröffentlichung und Standalone-Binary folgen separat.
- **SDK:** Vor einem Folgefeature muss geklärt werden, welches konkrete Flue-SDK-Paket und welche API verwendet werden sollen. Das blockiert die erste Oberfläche nicht.
- **Sicherheit:** Loopback ist keine vollständige Sicherheitsgrenze. SvelteKit-Origin-Prüfungen für schreibende Requests beibehalten; keine Endpunkte für beliebige Pfade oder Shell-Kommandos anbieten.

## Geschätzte Komplexität

**Mittel** – die Oberfläche ist bewusst klein. Der wesentliche Integrationsaufwand liegt im verlässlichen Bun-Produktionsstart, dem projektbezogenen CLI-Kontext und einer sicheren, getesteten Ersteinrichtung.
