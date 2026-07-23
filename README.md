# Metrics — Modulares Monitoring-System in Rust

Metrics ist ein fully-functional, containerisiertes, modulares Monitoring-System geschrieben in 100% Rust. Es integriert eine eingebettete SurrealDB-Datenbank (unter Verwendung der persistenten, MVCC-fähigen SurrealKV Storage-Engine), dynamic library Plugin-Lade-Unterstützung via `libloading` und einen Axum-API-Server, der ein neon-accentuiertes, glassmorphes Single Page Application Dashboard bereitstellt.

Ziel ist es, Metriken aus verschiedenen Quellen (ZFS, Systemressourcen, Immich, etc.) zu sammeln und direkt in Prometheus (via Pushgateway) oder InfluxDB (Line Protocol) zu schreiben.

---

## 🏗️ Architektur & Verzeichnisstruktur

Das Projekt ist als Cargo-Workspace strukturiert:

*   **`Cargo.toml`**: Workspace-Konfiguration.
*   **`metrics-api/`**: Shared API Crate, das die Module und die `MonitorModule` Trait Schnittstellen definiert:
    *   Definiert das `MonitorModule` Trait, `DbBackend` Konfigurationen und die `write_metrics` Hilfsfunktion.
*   **`metrics-core/`**: Die zentrale Engine und der Scheduler-Koordinator:
    *   `src/main.rs`: Initialisiert Einstellungen, DB, UI-Router und Scheduler.
    *   `src/db.rs`: Verwaltet ACID-konforme SurrealDB/SurrealKV Schreib- und Lesezugriffe.
    *   `src/loader.rs`: Lädt und entlädt Plugin-Bibliotheken (`.so` / `.dll`) zur Laufzeit.
    *   `src/scheduler.rs`: Task-Scheduler, der Module im gewünschten Intervall ausführt.
    *   `src/api.rs`: Bietet JWT-Authentifizierung und die Endpoint-Kompilierung für neue Plugins.
    *   `src/static/index.html`: Poliertes, responsives Dashboard mit Live-Logs und Einstellungsformularen.
*   **`modules/`**: Curated Plugins:
    *   **`metrics-system/`**: Überwacht CPU, RAM und Festplattenbelegung via `sysinfo`.
    *   **`metrics-zfs/`**: Führt `zpool list` aus, um den Pool-Status und die Speichernutzung zu erfassen.
    *   **`metrics-immich/`**: Ruft Statistiken (Fotos, Videos, Speicher) von der Immich-API ab.
    *   *Hinweis: Alle Module besitzen einen `mock`-Modus für schnelles lokales Testen.*
*   **`.github/workflows/`**: Continuous Integration Workflow, der das Docker-Image bei Pushes auf `main` baut und an GitHub Container Registry (GHCR) pusht.

---

## 🚀 Deployment & Ausführung

### Option A: Schnelles Starten mit Docker Compose (Empfohlen)

Das docker-compose Setup lädt automatisch das vorkompilierte Image aus der GitHub Container Registry herunter:

1.  **Container im Hintergrund starten**:
    ```bash
    docker compose up -d
    ```
2.  **Dashboard aufrufen**:
    *   Öffnen Sie: [http://localhost:3000](http://localhost:3000)
    *   Anmelden mit: Benutzername `admin` und Standard-Passwort `admin123` (Konfigurierbar via `ADMIN_PASSWORD` in `docker-compose.yml`).

### Option B: Lokales Kompilieren und Starten (Entwickler)

1.  **Workspace bauen**:
    ```bash
    cargo build --release
    ```
2.  **Server starten**:
    *   **Windows**:
        ```powershell
        $env:PORT="3000"; $env:ADMIN_PASSWORD="admin123"; $env:DB_PATH="./data"; .\target\release\metrics-core.exe
        ```
    *   **Linux**:
        ```bash
        PORT=3000 ADMIN_PASSWORD=admin123 DB_PATH=./data ./target/release/metrics-core
        ```

---

## 🧩 Eigene Module erstellen (HACS-Style)

Jedes Modul ist ein eigenständiges Rust-Crate, das die Shared Trait aus `metrics-api` implementiert.

1.  Erstellen Sie ein neues Cargo-Bibliotheksprojekt.
2.  Integrieren Sie `metrics-api` als Abhängigkeit.
3.  Fügen Sie in Ihrer `Cargo.toml` hinzu:
    ```toml
    [lib]
    crate-type = ["cdylib"]
    ```
4.  Implementieren Sie das `MonitorModule`-Schnittstellentrait und exportieren Sie die Erstellungsmethode:
    ```rust
    #[no_mangle]
    #[allow(improper_ctypes_definitions)]
    pub extern "C" fn create_module() -> *mut dyn MonitorModule {
        let module = MyCustomModule::default();
        let boxed = Box::new(module) as Box<dyn MonitorModule>;
        Box::into_raw(boxed)
    }
    ```
5.  Installieren Sie das Modul über die Web-UI, indem Sie die ID und Ihre Git-Repository-URL im Store eintragen. Metrics klont, kompiliert und lädt Ihr Modul vollautomatisch im laufenden Betrieb!
