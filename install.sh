#!/usr/bin/env bash

set -Eeuo pipefail

REPOSITORY="lgse/strata"
APP_ID="io.github.lgse.Strata"
MIN_GLIBC="2.39"
REQUIRED_PACKAGES=(
  bubblewrap desktop-file-utils ffmpeg ffmpegthumbnailer fontconfig gst-libav gstreamer
  gst-plugins-base gst-plugins-good gtk4 gtksourceview5 gvfs poppler-glib xdg-utils
)
RAW_PREVIEW_PACKAGES=(imagemagick libraw dcraw)
# Raw Ed25519 public keys; keep in sync with data/update-keys.json, the updater's trust store.
RELEASE_PUBLIC_KEYS=(
  1589903225a2dd3c1bc87125884801552b07fd0a09855cfdbde3b84601cebf3c
)
MANIFEST_NAME="strata-update-manifest.json"
SIGNATURES_NAME="strata-update-manifest.signatures.json"
MAX_MANIFEST_BYTES=32768
MAX_SIGNATURES_BYTES=8192
MAX_ARCHIVE_BYTES=$((128 * 1024 * 1024))
SIGNED_ARCHIVE_SIZE=""
SIGNED_ARCHIVE_SHA256=""

NON_INTERACTIVE=no
VERBOSE=no
WITH_SMB=ask
WITH_RAW=ask
WITH_DESKTOP_ENTRY=ask
WITH_FOLDER_ASSOCIATION=ask
WITH_FILE_MANAGER=ask
WITH_FILE_CHOOSER=ask
WITH_OMARCHY_KEYBINDS=ask
WITH_UDISKIE_UNLOCK=ask
REQUESTED_FILE_CHOOSER=no
REQUESTED_UDISKIE_UNLOCK=no
COLOR_RESET="" COLOR_BOLD="" COLOR_DIM="" COLOR_ACCENT="" COLOR_SUCCESS="" COLOR_WARNING="" COLOR_ERROR=""
COLOR_LAYER_ONE="" COLOR_LAYER_TWO=""
RICH_TERMINAL=no
WARNING_COUNT=0
CORE_INSTALLED=no
SETUP_NOTICES=no
ACTIVE_PID=""
LOG_FILE=""
TEMP_DIR=""

installer_locale() {
  local messages=${LC_ALL:-${LC_MESSAGES:-${LANG:-C}}} candidate
  local -a candidates
  if [[ $messages == C || $messages == POSIX ]]; then
    printf en
    return
  fi
  IFS=: read -ra candidates <<<"${LANGUAGE:-}"
  candidates+=("$messages")
  for candidate in "${candidates[@]}"; do
    case ${candidate,,} in
      en* | c | c.* | posix*) printf en; return ;;
      fr*) printf fr; return ;;
      de*) printf de; return ;;
      hu*) printf hu; return ;;
      es*) printf es; return ;;
      ja*) printf ja; return ;;
      pt*) printf pt-BR; return ;;
      ko*) printf ko; return ;;
      vi*) printf vi; return ;;
      it*) printf it; return ;;
      ru*) printf ru; return ;;
      zh[-_]hant* | zh[-_]tw* | zh[-_]hk* | zh[-_]mo*) printf zh-TW; return ;;
      zh*) printf zh-CN; return ;;
    esac
  done
  printf en
}

# installer-translations:start
# Generated from data/locales/installer.json by scripts/installer_messages.py.
declare -A INSTALLER_MESSAGES=()
case "$(installer_locale)" in
  en) INSTALLER_MESSAGES=(
    [setup]='Interactive installer'
    [tagline]='Navigate every layer.'
    [partial]='Strata is installed, but setup did not finish.'
    [failed]='Installation did not finish.'
    [details]='Detailed output: %{path}'
    [cancelled]='Installation cancelled. No changes made.'
    [task_failed]='Failed: %{task}'
    [read_failed]='Could not read your selection.'
    [keys_toggle]='↑/↓ move · Space toggle · Enter install · Esc cancel'
    [keys_choose]='↑/↓ choose · Enter continue · Esc cancel'
    [plain_toggle]='Toggle an option by number; Enter to install, q to cancel:'
    [plain_choose]='Choose [1], or q to cancel:'
    [invalid_selection]='Choose one of the listed options.'
    [standard]='Install Strata (recommended)'
    [custom]='Customize installation'
    [standard_hint]='Everything this system supports: launcher, folders, file chooser, and optional integrations.'
    [custom_hint]='Choose desktop integrations and optional support.'
    [setup_title]='Choose your setup'
    [launcher]='Application launcher'
    [folders]='Default file manager'
    [reveal]='Open file location'
    [chooser]='Open and Save dialogs'
    [launcher_hint]='Add Strata to your application menu.'
    [folders_hint]='Use Strata for folders and Open file location. Includes the launcher.'
    [reveal_hint]='Reveal files from other apps without changing folder defaults.'
    [reveal_ready]='Strata now handles "Open file location" requests.'
    [reveal_relogin]='Installed the FileManager1 service, but could not reload the session bus. Log out and back in to use Strata for "Open file location" requests.'
    [chooser_hint]='Use Strata for portal dialogs. Restarts the portal service; close file dialogs first.'
    [smb]='Windows network shares'
    [smb_hint]='Install optional SMB support for browsing network shares.'
    [raw]='Camera RAW support'
    [raw_hint]='Install optional image decoders for broader preview support.'
    [omarchy]='Omarchy file-manager shortcuts'
    [omarchy_hint]='Replace the file-manager shortcuts. A backup is kept.'
    [udiskie]='Encrypted-drive unlock'
    [udiskie_hint]='Edit udiskie settings, disable LUKS automount, and restart udiskie. Restore with strata --uninstall-udiskie-unlock.'
    [custom_title]='Make Strata yours'
    [plan]='Installation plan'
    [destination]='Install to %{path}'
    [defaults_unchanged]='Your default apps and shortcuts will stay unchanged.'
    [dependencies_ready]='All selected runtime packages are already installed.'
    [dependencies]='Install dependencies'
    [packages]='Packages to install: %{packages}'
    [reinstall]='This replaces %{path}. Your saved settings are kept.'
    [installing]='Installing Strata'
    [download]='Download Strata %{version}'
    [download_manifest]='Download the signed release manifest'
    [download_signatures]='Download the release signatures'
    [signature]='Verify the release signature'
    [digest]='Verify the signed SHA-256 digest'
    [unpack]='Unpack the verified release'
    [application]='Install the application'
    [ready]='Ready to explore'
    [installed_notices]='Installed — review setup details'
    [installed_warnings]='Installed with warnings'
    [installed]='Strata %{version} is installed.'
    [launch]='Run %{command} to get started.'
    [usage_intro]='Without options, choose a recommended install or customize desktop integration.'
    [verbose_help]='Show detailed download and verification output'
    [udiskie_requires]='--with-udiskie-unlock requires an Arch-based system with udiskie on PATH.'
    [progress]='%{percent}%'
  ) ;;
  hu) INSTALLER_MESSAGES=(
    [setup]='Interaktív telepítő'
    [tagline]='Fedezzen fel minden réteget.'
    [partial]='A Strata telepítve van, de a beállítás nem fejeződött be.'
    [failed]='A telepítés nem fejeződött be.'
    [details]='Részletes kimenet: %{path}'
    [cancelled]='A telepítés megszakítva. Nem történt módosítás.'
    [task_failed]='Sikertelen: %{task}'
    [read_failed]='Nem sikerült beolvasni a választását.'
    [keys_toggle]='↑/↓ mozgás · Szóköz be/ki · Enter telepítés · Esc mégse'
    [keys_choose]='↑/↓ választás · Enter tovább · Esc mégse'
    [plain_toggle]='Kapcsoljon be vagy ki egy lehetőséget a számával; Enter a telepítéshez, q a megszakításhoz:'
    [plain_choose]='Válasszon: [1], vagy q a megszakításhoz:'
    [invalid_selection]='Válasszon a felsorolt lehetőségek közül.'
    [standard]='A Strata telepítése (ajánlott)'
    [custom]='Telepítés testreszabása'
    [standard_hint]='Minden, amit ez a rendszer támogat: indító, mappák, fájlválasztó és opcionális integrációk.'
    [custom_hint]='Válassza ki az asztali integrációkat és az opcionális támogatást.'
    [setup_title]='Válassza ki a beállítást'
    [launcher]='Alkalmazásindító'
    [folders]='Alapértelmezett fájlkezelő'
    [reveal]='Fájl helyének megnyitása'
    [chooser]='Megnyitási és mentési párbeszédablakok'
    [launcher_hint]='A Strata hozzáadása az alkalmazásmenühöz.'
    [folders_hint]='A Strata használata mappákhoz és a „Fájl helyének megnyitása” művelethez. Tartalmazza az indítót is.'
    [reveal_hint]='Fájlok megjelenítése más alkalmazásokból a mappák alapértelmezett kezelőjének módosítása nélkül.'
    [reveal_ready]='A Strata mostantól kezeli a „Fájl helyének megnyitása” kéréseket.'
    [reveal_relogin]='A FileManager1 szolgáltatás telepítve lett, de a munkamenetbuszt nem sikerült újratölteni. Jelentkezzen ki, majd be újra, hogy a Strata kezelje a „Fájl helyének megnyitása” kéréseket.'
    [chooser_hint]='A Strata használata a portál párbeszédablakaihoz. Újraindítja a portálszolgáltatást; előbb zárja be a fájlpárbeszédablakokat.'
    [smb]='Windows hálózati megosztások'
    [smb_hint]='Opcionális SMB-támogatás telepítése a hálózati megosztások böngészéséhez.'
    [raw]='Fényképezőgépes RAW-támogatás'
    [raw_hint]='Opcionális képdekóderek telepítése a szélesebb körű előnézeti támogatáshoz.'
    [omarchy]='Omarchy fájlkezelő-gyorsbillentyűk'
    [omarchy_hint]='A fájlkezelő gyorsbillentyűinek lecserélése. Biztonsági másolat készül.'
    [udiskie]='Titkosított meghajtók feloldása'
    [udiskie_hint]='Az udiskie beállításainak szerkesztése, a LUKS automatikus csatolásának letiltása és az udiskie újraindítása. Visszaállítás: strata --uninstall-udiskie-unlock.'
    [custom_title]='Szabja testre a Stratát'
    [plan]='Telepítési terv'
    [destination]='Telepítés ide: %{path}'
    [defaults_unchanged]='Az alapértelmezett alkalmazások és gyorsbillentyűk változatlanok maradnak.'
    [dependencies_ready]='Az összes kiválasztott futtatókörnyezeti csomag már telepítve van.'
    [dependencies]='Függőségek telepítése'
    [packages]='Telepítendő csomagok: %{packages}'
    [reinstall]='Ez lecseréli a következőt: %{path}. A mentett beállításai megmaradnak.'
    [installing]='A Strata telepítése'
    [download]='A Strata %{version} letöltése'
    [download_manifest]='Az aláírt kiadási jegyzék letöltése'
    [download_signatures]='A kiadás aláírásainak letöltése'
    [signature]='A kiadás aláírásának ellenőrzése'
    [digest]='Az aláírt SHA-256 kivonat ellenőrzése'
    [unpack]='Az ellenőrzött kiadás kicsomagolása'
    [application]='Az alkalmazás telepítése'
    [ready]='Indulhat a felfedezés'
    [installed_notices]='Telepítve — tekintse át a beállítás részleteit'
    [installed_warnings]='Telepítve, figyelmeztetésekkel'
    [installed]='A Strata %{version} telepítve van.'
    [launch]='Az indításhoz futtassa a következőt: %{command}'
    [usage_intro]='Kapcsolók nélkül választhat az ajánlott telepítés és az asztali integráció testreszabása között.'
    [verbose_help]='Részletes letöltési és ellenőrzési kimenet megjelenítése'
    [udiskie_requires]='A --with-udiskie-unlock kapcsolóhoz Arch-alapú rendszer szükséges, amelynek PATH változójában elérhető az udiskie.'
    [progress]='%{percent}%'
  ) ;;
  fr) INSTALLER_MESSAGES=(
    [setup]='Programme d’installation interactif'
    [tagline]='Parcourez chaque couche.'
    [partial]='Strata est installé, mais la configuration n’est pas terminée.'
    [failed]='L’installation n’est pas terminée.'
    [details]='Journal détaillé : %{path}'
    [cancelled]='Installation annulée. Aucune modification effectuée.'
    [task_failed]='Échec : %{task}'
    [read_failed]='Impossible de lire votre choix.'
    [keys_toggle]='↑/↓ déplacer · Espace cocher · Entrée installer · Échap annuler'
    [keys_choose]='↑/↓ choisir · Entrée continuer · Échap annuler'
    [plain_toggle]='Cochez par numéro ; Entrée pour installer, q pour annuler :'
    [plain_choose]='Choisissez [1], ou q pour annuler :'
    [invalid_selection]='Choisissez l’une des options proposées.'
    [standard]='Installer Strata (recommandé)'
    [custom]='Personnaliser l’installation'
    [standard_hint]='Tout ce que ce système prend en charge : lanceur, dossiers, sélecteur de fichiers et intégrations optionnelles.'
    [custom_hint]='Choisissez l’intégration au bureau et les fonctions facultatives.'
    [setup_title]='Choisissez votre installation'
    [launcher]='Lanceur d’application'
    [folders]='Gestionnaire de fichiers par défaut'
    [reveal]='Ouvrir l’emplacement du fichier'
    [chooser]='Boîtes de dialogue Ouvrir et Enregistrer'
    [launcher_hint]='Ajoutez Strata au menu des applications.'
    [folders_hint]='Utilisez Strata pour les dossiers et leur ouverture depuis d’autres applications. Lanceur inclus.'
    [reveal_hint]='Affichez les fichiers depuis d’autres applications sans changer le gestionnaire de dossiers.'
    [reveal_ready]='Strata gère désormais les demandes « Ouvrir l’emplacement du fichier ».'
    [reveal_relogin]='Le service FileManager1 est installé, mais le bus de session n’a pas pu être rechargé. Déconnectez-vous puis reconnectez-vous pour utiliser Strata pour les demandes « Ouvrir l’emplacement du fichier ».'
    [chooser_hint]='Utilisez Strata pour les dialogues de portail. Redémarre le service ; fermez d’abord les dialogues de fichiers.'
    [smb]='Partages réseau Windows'
    [smb_hint]='Installez la prise en charge SMB pour parcourir les partages réseau.'
    [raw]='Prise en charge des photos RAW'
    [raw_hint]='Installez des décodeurs d’images pour élargir la prise en charge des aperçus.'
    [omarchy]='Raccourcis du gestionnaire de fichiers Omarchy'
    [omarchy_hint]='Remplacez les raccourcis du gestionnaire de fichiers. Une sauvegarde est conservée.'
    [udiskie]='Déverrouillage des disques chiffrés'
    [udiskie_hint]='Modifiez udiskie, désactivez le montage automatique LUKS et redémarrez udiskie. Restaurez avec strata --uninstall-udiskie-unlock.'
    [custom_title]='Adaptez Strata à vos besoins'
    [plan]='Plan d’installation'
    [destination]='Installer dans %{path}'
    [defaults_unchanged]='Vos applications par défaut et vos raccourcis restent inchangés.'
    [dependencies_ready]='Tous les paquets d’exécution sélectionnés sont déjà installés.'
    [dependencies]='Installer les dépendances'
    [packages]='Paquets à installer : %{packages}'
    [reinstall]='Cette opération remplace %{path}. Vos réglages sont conservés.'
    [installing]='Installation de Strata'
    [download]='Télécharger Strata %{version}'
    [download_manifest]='Télécharger le manifeste signé de la version'
    [download_signatures]='Télécharger les signatures de la version'
    [signature]='Vérifier la signature de la version'
    [digest]='Vérifier l'"'"'empreinte SHA-256 signée'
    [unpack]='Décompresser la version vérifiée'
    [application]='Installer l’application'
    [ready]='Tout est prêt pour explorer'
    [installed_notices]='Installé — consultez les détails de configuration'
    [installed_warnings]='Installation terminée avec avertissements'
    [installed]='Strata %{version} est installé.'
    [launch]='Exécutez %{command} pour commencer.'
    [usage_intro]='Sans option, choisissez l’installation recommandée ou personnalisez l’intégration au bureau.'
    [verbose_help]='Afficher les détails du téléchargement et de la vérification'
    [udiskie_requires]='--with-udiskie-unlock nécessite un système basé sur Arch avec udiskie dans PATH.'
    [progress]='%{percent} %'
  ) ;;
  de) INSTALLER_MESSAGES=(
    [setup]='Interaktives Installationsprogramm'
    [tagline]='Jede Ebene erkunden.'
    [partial]='Strata ist installiert, aber die Einrichtung wurde nicht abgeschlossen.'
    [failed]='Die Installation wurde nicht abgeschlossen.'
    [details]='Ausführliches Protokoll: %{path}'
    [cancelled]='Installation abgebrochen. Es wurden keine Änderungen vorgenommen.'
    [task_failed]='Fehlgeschlagen: %{task}'
    [read_failed]='Die Auswahl konnte nicht gelesen werden.'
    [keys_toggle]='↑/↓ bewegen · Leertaste umschalten · Eingabe installieren · Esc abbrechen'
    [keys_choose]='↑/↓ auswählen · Eingabe fortfahren · Esc abbrechen'
    [plain_toggle]='Option per Nummer umschalten; Eingabe zum Installieren, q zum Abbrechen:'
    [plain_choose]='Auswahl [1], oder q zum Abbrechen:'
    [invalid_selection]='Wählen Sie eine der aufgeführten Optionen.'
    [standard]='Strata installieren (empfohlen)'
    [custom]='Installation anpassen'
    [standard_hint]='Alles, was dieses System unterstützt: Starter, Ordner, Dateiauswahl und optionale Integrationen.'
    [custom_hint]='Desktop-Integration und optionale Unterstützung auswählen.'
    [setup_title]='Einrichtung auswählen'
    [launcher]=Anwendungsstarter
    [folders]=Standard-Dateimanager
    [reveal]='Dateispeicherort öffnen'
    [chooser]='Öffnen- und Speichern-Dialoge'
    [launcher_hint]='Strata zum Anwendungsmenü hinzufügen.'
    [folders_hint]='Strata für Ordner und das Öffnen von Dateispeicherorten verwenden. Starter inklusive.'
    [reveal_hint]='Dateien aus anderen Anwendungen anzeigen, ohne den Standard für Ordner zu ändern.'
    [reveal_ready]='Strata verarbeitet jetzt Anfragen für „Dateispeicherort öffnen“.'
    [reveal_relogin]='Der FileManager1-Dienst wurde installiert, aber der Sitzungsbus konnte nicht neu geladen werden. Melden Sie sich ab und wieder an, um Strata für „Dateispeicherort öffnen“ zu verwenden.'
    [chooser_hint]='Strata für Portal-Dialoge verwenden. Startet den Portal-Dienst neu; Dateidialoge vorher schließen.'
    [smb]=Windows-Netzwerkfreigaben
    [smb_hint]='Optionale SMB-Unterstützung zum Durchsuchen von Netzwerkfreigaben installieren.'
    [raw]='Kamera-RAW-Unterstützung'
    [raw_hint]='Optionale Bilddecoder für zusätzliche Vorschauformate installieren.'
    [omarchy]='Omarchy-Dateimanager-Tastenkürzel'
    [omarchy_hint]='Dateimanager-Tastenkürzel ersetzen. Eine Sicherung wird erstellt.'
    [udiskie]='Verschlüsselte Laufwerke entsperren'
    [udiskie_hint]='udiskie konfigurieren, LUKS-Automount deaktivieren und udiskie neu starten. Mit strata --uninstall-udiskie-unlock wiederherstellen.'
    [custom_title]='Strata nach Ihren Wünschen'
    [plan]='Installationsübersicht'
    [destination]='Installation nach %{path}'
    [defaults_unchanged]='Ihre Standardanwendungen und Tastenkürzel bleiben unverändert.'
    [dependencies_ready]='Alle ausgewählten Laufzeitpakete sind bereits installiert.'
    [dependencies]='Abhängigkeiten installieren'
    [packages]='Zu installierende Pakete: %{packages}'
    [reinstall]='%{path} wird ersetzt. Ihre gespeicherten Einstellungen bleiben erhalten.'
    [installing]='Strata wird installiert'
    [download]='Strata %{version} herunterladen'
    [download_manifest]='Signiertes Versionsmanifest herunterladen'
    [download_signatures]='Signaturen der Version herunterladen'
    [signature]='Signatur der Version überprüfen'
    [digest]='Signierten SHA-256-Hash überprüfen'
    [unpack]='Überprüfte Version entpacken'
    [application]='Anwendung installieren'
    [ready]='Bereit zum Erkunden'
    [installed_notices]='Installiert — Einrichtungshinweise prüfen'
    [installed_warnings]='Mit Warnungen installiert'
    [installed]='Strata %{version} ist installiert.'
    [launch]='Starten Sie mit %{command}.'
    [usage_intro]='Ohne Optionen können Sie die empfohlene Installation wählen oder die Desktop-Integration anpassen.'
    [verbose_help]='Ausführliche Download- und Prüfausgabe anzeigen'
    [udiskie_requires]='--with-udiskie-unlock erfordert ein Arch-basiertes System mit udiskie im PATH.'
    [progress]='%{percent} %'
  ) ;;
  es) INSTALLER_MESSAGES=(
    [setup]='Instalador interactivo'
    [tagline]='Navega por cada capa.'
    [partial]='Strata está instalado, pero la configuración no terminó.'
    [failed]='La instalación no terminó.'
    [details]='Registro detallado: %{path}'
    [cancelled]='Instalación cancelada. No se hicieron cambios.'
    [task_failed]='Error: %{task}'
    [read_failed]='No se pudo leer tu selección.'
    [keys_toggle]='↑/↓ mover · Espacio marcar · Intro instalar · Esc cancelar'
    [keys_choose]='↑/↓ elegir · Intro continuar · Esc cancelar'
    [plain_toggle]='Marca una opción por número; Intro para instalar, q para cancelar:'
    [plain_choose]='Elige [1], o q para cancelar:'
    [invalid_selection]='Elige una de las opciones de la lista.'
    [standard]='Instalar Strata (recomendado)'
    [custom]='Personalizar la instalación'
    [standard_hint]='Todo lo que admite este sistema: lanzador, carpetas, selector de archivos e integraciones opcionales.'
    [custom_hint]='Elige la integración con el escritorio y las funciones opcionales.'
    [setup_title]='Elige tu instalación'
    [launcher]='Lanzador de aplicaciones'
    [folders]='Gestor de archivos predeterminado'
    [reveal]='Abrir ubicación del archivo'
    [chooser]='Diálogos de abrir y guardar'
    [launcher_hint]='Añade Strata al menú de aplicaciones.'
    [folders_hint]='Usa Strata para carpetas y para abrir la ubicación de archivos. Incluye el lanzador.'
    [reveal_hint]='Muestra archivos desde otras aplicaciones sin cambiar el gestor de carpetas predeterminado.'
    [reveal_ready]='Strata ahora gestiona las solicitudes de «Abrir ubicación del archivo».'
    [reveal_relogin]='Se instaló el servicio FileManager1, pero no se pudo recargar el bus de sesión. Cierra sesión y vuelve a iniciarla para usar Strata en las solicitudes de «Abrir ubicación del archivo».'
    [chooser_hint]='Usa Strata para los diálogos del portal. Reinicia el servicio; cierra antes los diálogos de archivos.'
    [smb]='Recursos compartidos de Windows'
    [smb_hint]='Instala el soporte SMB opcional para explorar recursos compartidos de red.'
    [raw]='Soporte para fotos RAW'
    [raw_hint]='Instala decodificadores de imágenes opcionales para ampliar las vistas previas.'
    [omarchy]='Atajos del gestor de archivos de Omarchy'
    [omarchy_hint]='Reemplaza los atajos del gestor de archivos. Se conserva una copia de seguridad.'
    [udiskie]='Desbloqueo de unidades cifradas'
    [udiskie_hint]='Configura udiskie, desactiva el montaje automático LUKS y reinicia udiskie. Restaura con strata --uninstall-udiskie-unlock.'
    [custom_title]='Haz Strata a tu medida'
    [plan]='Plan de instalación'
    [destination]='Instalar en %{path}'
    [defaults_unchanged]='Tus aplicaciones predeterminadas y atajos no cambiarán.'
    [dependencies_ready]='Todos los paquetes de ejecución seleccionados ya están instalados.'
    [dependencies]='Instalar dependencias'
    [packages]='Paquetes que se instalarán: %{packages}'
    [reinstall]='Se reemplazará %{path}. Se conservarán tus ajustes guardados.'
    [installing]='Instalando Strata'
    [download]='Descargar Strata %{version}'
    [download_manifest]='Descargar el manifiesto firmado de la versión'
    [download_signatures]='Descargar las firmas de la versión'
    [signature]='Verificar la firma de la versión'
    [digest]='Verificar el resumen SHA-256 firmado'
    [unpack]='Extraer la versión verificada'
    [application]='Instalar la aplicación'
    [ready]='Todo listo para explorar'
    [installed_notices]='Instalado — revisa los detalles de configuración'
    [installed_warnings]='Instalado con advertencias'
    [installed]='Strata %{version} está instalado.'
    [launch]='Ejecuta %{command} para comenzar.'
    [usage_intro]='Sin opciones, elige la instalación recomendada o personaliza la integración con el escritorio.'
    [verbose_help]='Mostrar detalles de descarga y verificación'
    [udiskie_requires]='--with-udiskie-unlock requiere un sistema basado en Arch con udiskie en PATH.'
    [progress]='%{percent} %'
  ) ;;
  ja) INSTALLER_MESSAGES=(
    [setup]='対話式インストーラー'
    [tagline]='すべての階層をナビゲート。'
    [partial]='Strata はインストール済みですが、設定は完了していません。'
    [failed]='インストールは完了していません。'
    [details]='詳細ログ: %{path}'
    [cancelled]='インストールをキャンセルしました。変更は行っていません。'
    [task_failed]='失敗: %{task}'
    [read_failed]='選択内容を読み取れませんでした。'
    [keys_toggle]='↑/↓ 移動 · Space 切替 · Enter インストール · Esc キャンセル'
    [keys_choose]='↑/↓ 選択 · Enter 続行 · Esc キャンセル'
    [plain_toggle]='番号で切替、Enter でインストール、q でキャンセル:'
    [plain_choose]='選択 [1]、q でキャンセル:'
    [invalid_selection]='一覧の選択肢から選んでください。'
    [standard]='Strata をインストール（推奨）'
    [custom]='インストールをカスタマイズ'
    [standard_hint]='このシステムが対応するすべて: ランチャー、フォルダー、ファイル選択ダイアログ、オプションの統合。'
    [custom_hint]='デスクトップ連携と追加機能を選びます。'
    [setup_title]='インストール方法を選択'
    [launcher]='アプリランチャー'
    [folders]='既定のファイルマネージャー'
    [reveal]='ファイルの場所を開く'
    [chooser]='開く・保存ダイアログ'
    [launcher_hint]='アプリケーションメニューに Strata を追加します。'
    [folders_hint]='フォルダーやファイルの場所を Strata で開きます。ランチャーも追加します。'
    [reveal_hint]='フォルダーの既定設定を変えずに、他のアプリからファイルの場所を表示します。'
    [reveal_ready]='Strata が「ファイルの場所を開く」要求を処理するようになりました。'
    [reveal_relogin]='FileManager1 サービスをインストールしましたが、セッションバスを再読み込みできませんでした。「ファイルの場所を開く」要求に Strata を使うには、ログアウトして再度ログインしてください。'
    [chooser_hint]='ポータルのダイアログに Strata を使用します。サービスを再起動するため、先にファイルダイアログを閉じてください。'
    [smb]='Windows ネットワーク共有'
    [smb_hint]='ネットワーク共有を参照するための SMB サポートを追加します。'
    [raw]='カメラ RAW サポート'
    [raw_hint]='より多くの画像をプレビューできるよう、追加のデコーダーをインストールします。'
    [omarchy]='Omarchy のファイルマネージャーショートカット'
    [omarchy_hint]='ファイルマネージャーのショートカットを置き換えます。バックアップを保存します。'
    [udiskie]='暗号化ドライブのロック解除'
    [udiskie_hint]='udiskie の設定を変更し、LUKS の自動マウントを無効にして再起動します。strata --uninstall-udiskie-unlock で元に戻せます。'
    [custom_title]='Strata を自分好みに'
    [plan]='インストール内容'
    [destination]='インストール先: %{path}'
    [defaults_unchanged]='既定のアプリとショートカットは変更しません。'
    [dependencies_ready]='選択した実行時パッケージはすべてインストール済みです。'
    [dependencies]='依存パッケージをインストール'
    [packages]='インストールするパッケージ: %{packages}'
    [reinstall]='%{path} を置き換えます。保存済みの設定は保持します。'
    [installing]='Strata をインストール中'
    [download]='Strata %{version} をダウンロード'
    [download_manifest]='署名済みリリースマニフェストをダウンロード'
    [download_signatures]='リリースの署名をダウンロード'
    [signature]='リリースの署名を検証'
    [digest]='署名済み SHA-256 ダイジェストを検証'
    [unpack]='検証済みリリースを展開'
    [application]='アプリをインストール'
    [ready]='準備ができました'
    [installed_notices]='インストール済み — 設定の詳細を確認してください'
    [installed_warnings]='警告付きでインストールしました'
    [installed]='Strata %{version} をインストールしました。'
    [launch]='%{command} を実行して使い始めましょう。'
    [usage_intro]='オプションなしでは、推奨インストールかデスクトップ連携のカスタマイズを選べます。'
    [verbose_help]='ダウンロードと検証の詳細を表示'
    [udiskie_requires]='--with-udiskie-unlock には、PATH 上に udiskie がある Arch 系システムが必要です。'
    [progress]='%{percent}%'
  ) ;;
  pt-BR) INSTALLER_MESSAGES=(
    [setup]='Instalador interativo'
    [tagline]='Navegue por todas as camadas.'
    [partial]='O Strata está instalado, mas a configuração não foi concluída.'
    [failed]='A instalação não foi concluída.'
    [details]='Registro detalhado: %{path}'
    [cancelled]='Instalação cancelada. Nenhuma alteração foi feita.'
    [task_failed]='Falha: %{task}'
    [read_failed]='Não foi possível ler sua seleção.'
    [keys_toggle]='↑/↓ mover · Espaço marcar · Enter instalar · Esc cancelar'
    [keys_choose]='↑/↓ escolher · Enter continuar · Esc cancelar'
    [plain_toggle]='Marque uma opção pelo número; Enter para instalar, q para cancelar:'
    [plain_choose]='Escolha [1], ou q para cancelar:'
    [invalid_selection]='Escolha uma das opções listadas.'
    [standard]='Instalar o Strata (recomendado)'
    [custom]='Personalizar a instalação'
    [standard_hint]='Tudo o que este sistema suporta: lançador, pastas, seletor de arquivos e integrações opcionais.'
    [custom_hint]='Escolha integrações com a área de trabalho e recursos opcionais.'
    [setup_title]='Escolha sua instalação'
    [launcher]='Lançador de aplicativos'
    [folders]='Gerenciador de arquivos padrão'
    [reveal]='Abrir local do arquivo'
    [chooser]='Diálogos de abrir e salvar'
    [launcher_hint]='Adicione o Strata ao menu de aplicativos.'
    [folders_hint]='Use o Strata para pastas e para abrir locais de arquivos. Inclui o lançador.'
    [reveal_hint]='Mostre arquivos de outros aplicativos sem mudar o padrão para pastas.'
    [reveal_ready]='O Strata agora atende às solicitações de “Abrir local do arquivo”.'
    [reveal_relogin]='O serviço FileManager1 foi instalado, mas não foi possível recarregar o barramento de sessão. Saia e entre novamente para usar o Strata nas solicitações de “Abrir local do arquivo”.'
    [chooser_hint]='Use o Strata nos diálogos do portal. Reinicia o serviço; feche os diálogos de arquivos antes.'
    [smb]='Compartilhamentos de rede Windows'
    [smb_hint]='Instale o suporte SMB opcional para navegar em compartilhamentos de rede.'
    [raw]='Suporte a fotos RAW'
    [raw_hint]='Instale decodificadores de imagem opcionais para ampliar as prévias.'
    [omarchy]='Atalhos do gerenciador de arquivos do Omarchy'
    [omarchy_hint]='Substitua os atalhos do gerenciador de arquivos. Um backup é mantido.'
    [udiskie]='Desbloqueio de unidades criptografadas'
    [udiskie_hint]='Configure o udiskie, desative a montagem automática LUKS e reinicie o udiskie. Restaure com strata --uninstall-udiskie-unlock.'
    [custom_title]='Deixe o Strata do seu jeito'
    [plan]='Plano de instalação'
    [destination]='Instalar em %{path}'
    [defaults_unchanged]='Seus aplicativos padrão e atalhos não serão alterados.'
    [dependencies_ready]='Todos os pacotes de execução selecionados já estão instalados.'
    [dependencies]='Instalar dependências'
    [packages]='Pacotes a instalar: %{packages}'
    [reinstall]='%{path} será substituído. Suas configurações salvas serão mantidas.'
    [installing]='Instalando o Strata'
    [download]='Baixar o Strata %{version}'
    [download_manifest]='Baixar o manifesto assinado da versão'
    [download_signatures]='Baixar as assinaturas da versão'
    [signature]='Verificar a assinatura da versão'
    [digest]='Verificar o resumo SHA-256 assinado'
    [unpack]='Extrair a versão verificada'
    [application]='Instalar o aplicativo'
    [ready]='Tudo pronto para explorar'
    [installed_notices]='Instalado — confira os detalhes da configuração'
    [installed_warnings]='Instalado com avisos'
    [installed]='O Strata %{version} está instalado.'
    [launch]='Execute %{command} para começar.'
    [usage_intro]='Sem opções, escolha a instalação recomendada ou personalize a integração com a área de trabalho.'
    [verbose_help]='Mostrar detalhes do download e da verificação'
    [udiskie_requires]='--with-udiskie-unlock requer um sistema baseado em Arch com udiskie no PATH.'
    [progress]='%{percent}%'
  ) ;;
  ko) INSTALLER_MESSAGES=(
    [setup]='대화형 설치 프로그램'
    [tagline]='모든 계층을 탐색하세요.'
    [partial]='Strata는 설치되었지만 설정이 완료되지 않았습니다.'
    [failed]='설치가 완료되지 않았습니다.'
    [details]='상세 로그: %{path}'
    [cancelled]='설치를 취소했습니다. 변경 사항이 없습니다.'
    [task_failed]='실패: %{task}'
    [read_failed]='선택 항목을 읽을 수 없습니다.'
    [keys_toggle]='↑/↓ 이동 · Space 선택 · Enter 설치 · Esc 취소'
    [keys_choose]='↑/↓ 선택 · Enter 계속 · Esc 취소'
    [plain_toggle]='번호로 선택, Enter로 설치, q로 취소:'
    [plain_choose]='선택 [1], q로 취소:'
    [invalid_selection]='목록에서 항목을 선택하세요.'
    [standard]='Strata 설치 (권장)'
    [custom]='설치 사용자 지정'
    [standard_hint]='이 시스템이 지원하는 모든 항목: 런처, 폴더, 파일 선택기, 선택적 통합.'
    [custom_hint]='데스크톱 통합과 추가 지원을 선택합니다.'
    [setup_title]='설치 방식 선택'
    [launcher]='앱 실행 아이콘'
    [folders]='기본 파일 관리자'
    [reveal]='파일 위치 열기'
    [chooser]='열기 및 저장 대화상자'
    [launcher_hint]='앱 메뉴에 Strata를 추가합니다.'
    [folders_hint]='폴더와 파일 위치를 Strata로 엽니다. 실행 아이콘도 추가합니다.'
    [reveal_hint]='폴더 기본 설정을 바꾸지 않고 다른 앱에서 파일 위치를 표시합니다.'
    [reveal_ready]='이제 Strata가 "파일 위치 열기" 요청을 처리합니다.'
    [reveal_relogin]='FileManager1 서비스를 설치했지만 세션 버스를 다시 불러오지 못했습니다. "파일 위치 열기" 요청에 Strata를 사용하려면 로그아웃한 후 다시 로그인하세요.'
    [chooser_hint]='포털 대화상자에 Strata를 사용합니다. 포털 서비스가 재시작되므로 파일 대화상자를 먼저 닫으세요.'
    [smb]='Windows 네트워크 공유'
    [smb_hint]='네트워크 공유 탐색을 위한 SMB 지원을 추가합니다.'
    [raw]='카메라 RAW 지원'
    [raw_hint]='더 많은 이미지 미리보기를 위해 추가 디코더를 설치합니다.'
    [omarchy]='Omarchy 파일 관리자 단축키'
    [omarchy_hint]='파일 관리자 단축키를 바꿉니다. 백업은 보관됩니다.'
    [udiskie]='암호화 드라이브 잠금 해제'
    [udiskie_hint]='udiskie 설정을 바꾸고 LUKS 자동 마운트를 끈 뒤 재시작합니다. strata --uninstall-udiskie-unlock으로 복원합니다.'
    [custom_title]='나만의 Strata 설정'
    [plan]='설치 계획'
    [destination]='설치 위치: %{path}'
    [defaults_unchanged]='기본 앱과 단축키는 변경하지 않습니다.'
    [dependencies_ready]='선택한 실행 패키지가 모두 이미 설치되어 있습니다.'
    [dependencies]='의존 패키지 설치'
    [packages]='설치할 패키지: %{packages}'
    [reinstall]='%{path} 파일을 교체합니다. 저장된 설정은 유지됩니다.'
    [installing]='Strata 설치 중'
    [download]='Strata %{version} 다운로드'
    [download_manifest]='서명된 릴리스 매니페스트 다운로드'
    [download_signatures]='릴리스 서명 다운로드'
    [signature]='릴리스 서명 검증'
    [digest]='서명된 SHA-256 다이제스트 검증'
    [unpack]='검증된 릴리스 압축 풀기'
    [application]='앱 설치'
    [ready]='탐색할 준비가 되었습니다'
    [installed_notices]='설치됨 — 설정 세부 정보를 확인하세요'
    [installed_warnings]='경고와 함께 설치됨'
    [installed]='Strata %{version} 설치가 완료되었습니다.'
    [launch]='%{command} 명령으로 시작하세요.'
    [usage_intro]='옵션 없이 실행하면 권장 설치 또는 데스크톱 통합 사용자 지정을 선택합니다.'
    [verbose_help]='다운로드 및 검증 상세 출력 표시'
    [udiskie_requires]='--with-udiskie-unlock에는 PATH에 udiskie가 있는 Arch 기반 시스템이 필요합니다.'
    [progress]='%{percent}%'
  ) ;;
  vi) INSTALLER_MESSAGES=(
    [setup]='Trình cài đặt tương tác'
    [tagline]='Duyệt qua mọi lớp.'
    [partial]='Strata đã được cài đặt nhưng chưa hoàn tất thiết lập.'
    [failed]='Chưa hoàn tất cài đặt.'
    [details]='Nhật ký chi tiết: %{path}'
    [cancelled]='Đã hủy cài đặt. Không có thay đổi nào.'
    [task_failed]='Thất bại: %{task}'
    [read_failed]='Không thể đọc lựa chọn của bạn.'
    [keys_toggle]='↑/↓ di chuyển · Space chọn · Enter cài đặt · Esc hủy'
    [keys_choose]='↑/↓ chọn · Enter tiếp tục · Esc hủy'
    [plain_toggle]='Chọn theo số; Enter để cài đặt, q để hủy:'
    [plain_choose]='Chọn [1], hoặc q để hủy:'
    [invalid_selection]='Hãy chọn một trong các mục được liệt kê.'
    [standard]='Cài đặt Strata (đề xuất)'
    [custom]='Tùy chỉnh cài đặt'
    [standard_hint]='Mọi thứ hệ thống này hỗ trợ: trình khởi chạy, thư mục, hộp thoại chọn tệp và các tích hợp tùy chọn.'
    [custom_hint]='Chọn tích hợp môi trường làm việc và các tính năng bổ sung.'
    [setup_title]='Chọn cách cài đặt'
    [launcher]='Biểu tượng khởi chạy'
    [folders]='Trình quản lý tệp mặc định'
    [reveal]='Mở vị trí tệp'
    [chooser]='Hộp thoại mở và lưu'
    [launcher_hint]='Thêm Strata vào menu ứng dụng.'
    [folders_hint]='Dùng Strata để mở thư mục và vị trí tệp. Bao gồm biểu tượng khởi chạy.'
    [reveal_hint]='Hiện tệp từ ứng dụng khác mà không đổi ứng dụng mở thư mục mặc định.'
    [reveal_ready]='Strata giờ xử lý các yêu cầu “Mở vị trí tệp”.'
    [reveal_relogin]='Đã cài dịch vụ FileManager1 nhưng không thể tải lại bus phiên. Hãy đăng xuất rồi đăng nhập lại để dùng Strata cho các yêu cầu “Mở vị trí tệp”.'
    [chooser_hint]='Dùng Strata cho hộp thoại portal. Sẽ khởi động lại dịch vụ; hãy đóng các hộp thoại tệp trước.'
    [smb]='Chia sẻ mạng Windows'
    [smb_hint]='Cài hỗ trợ SMB tùy chọn để duyệt các thư mục chia sẻ mạng.'
    [raw]='Hỗ trợ ảnh RAW'
    [raw_hint]='Cài thêm bộ giải mã hình ảnh để xem trước nhiều định dạng hơn.'
    [omarchy]='Phím tắt trình quản lý tệp Omarchy'
    [omarchy_hint]='Thay phím tắt trình quản lý tệp. Giữ lại bản sao lưu.'
    [udiskie]='Mở khóa ổ đĩa mã hóa'
    [udiskie_hint]='Đổi thiết lập udiskie, tắt tự động gắn LUKS rồi khởi động lại udiskie. Khôi phục bằng strata --uninstall-udiskie-unlock.'
    [custom_title]='Thiết lập Strata theo ý bạn'
    [plan]='Kế hoạch cài đặt'
    [destination]='Cài đặt vào %{path}'
    [defaults_unchanged]='Giữ nguyên ứng dụng mặc định và phím tắt của bạn.'
    [dependencies_ready]='Tất cả gói chạy đã chọn đều được cài đặt sẵn.'
    [dependencies]='Cài các gói phụ thuộc'
    [packages]='Các gói sẽ cài đặt: %{packages}'
    [reinstall]='Sẽ thay thế %{path}. Giữ lại các thiết lập đã lưu.'
    [installing]='Đang cài đặt Strata'
    [download]='Tải Strata %{version}'
    [download_manifest]='Tải tệp kê khai bản phát hành đã ký'
    [download_signatures]='Tải chữ ký của bản phát hành'
    [signature]='Xác minh chữ ký của bản phát hành'
    [digest]='Xác minh mã băm SHA-256 đã ký'
    [unpack]='Giải nén bản phát hành đã xác minh'
    [application]='Cài đặt ứng dụng'
    [ready]='Sẵn sàng khám phá'
    [installed_notices]='Đã cài đặt — hãy xem chi tiết thiết lập'
    [installed_warnings]='Đã cài đặt với cảnh báo'
    [installed]='Đã cài đặt Strata %{version}.'
    [launch]='Chạy %{command} để bắt đầu.'
    [usage_intro]='Khi không có tùy chọn, chọn cài đặt đề xuất hoặc tùy chỉnh tích hợp môi trường làm việc.'
    [verbose_help]='Hiện chi tiết tải xuống và xác minh'
    [udiskie_requires]='--with-udiskie-unlock yêu cầu hệ thống dựa trên Arch có udiskie trong PATH.'
    [progress]='%{percent}%'
  ) ;;
  it) INSTALLER_MESSAGES=(
    [setup]='Programma di installazione interattivo'
    [tagline]='Naviga in ogni livello.'
    [partial]='Strata è installato, ma la configurazione non è stata completata.'
    [failed]='L’installazione non è stata completata.'
    [details]='Registro dettagliato: %{path}'
    [cancelled]='Installazione annullata. Nessuna modifica effettuata.'
    [task_failed]='Operazione non riuscita: %{task}'
    [read_failed]='Impossibile leggere la selezione.'
    [keys_toggle]='↑/↓ sposta · Spazio seleziona · Invio installa · Esc annulla'
    [keys_choose]='↑/↓ scegli · Invio continua · Esc annulla'
    [plain_toggle]='Seleziona per numero; Invio per installare, q per annullare:'
    [plain_choose]='Scegli [1], oppure q per annullare:'
    [invalid_selection]='Scegli una delle opzioni elencate.'
    [standard]='Installa Strata (consigliato)'
    [custom]='Personalizza l’installazione'
    [standard_hint]='Tutto ciò che questo sistema supporta: launcher, cartelle, selettore di file e integrazioni opzionali.'
    [custom_hint]='Scegli le integrazioni desktop e le funzionalità opzionali.'
    [setup_title]='Scegli la configurazione'
    [launcher]='Lanciatore dell’applicazione'
    [folders]='Gestore di file predefinito'
    [reveal]='Apri percorso del file'
    [chooser]='Finestre Apri e Salva'
    [launcher_hint]='Aggiungi Strata al menu delle applicazioni.'
    [folders_hint]='Usa Strata per cartelle e percorsi dei file. Include il lanciatore.'
    [reveal_hint]='Mostra file da altre applicazioni senza cambiare il gestore predefinito delle cartelle.'
    [reveal_ready]='Strata ora gestisce le richieste «Apri percorso del file».'
    [reveal_relogin]='Il servizio FileManager1 è stato installato, ma non è stato possibile ricaricare il bus di sessione. Esci e accedi di nuovo per usare Strata con le richieste «Apri percorso del file».'
    [chooser_hint]='Usa Strata per le finestre del portale. Riavvia il servizio; chiudi prima le finestre dei file.'
    [smb]='Condivisioni di rete Windows'
    [smb_hint]='Installa il supporto SMB opzionale per esplorare le condivisioni di rete.'
    [raw]='Supporto foto RAW'
    [raw_hint]='Installa decodificatori opzionali per visualizzare più formati di immagini.'
    [omarchy]='Scorciatoie del gestore di file Omarchy'
    [omarchy_hint]='Sostituisci le scorciatoie del gestore di file. Viene conservata una copia di backup.'
    [udiskie]='Sblocco delle unità cifrate'
    [udiskie_hint]='Configura udiskie, disattiva il montaggio automatico LUKS e riavvia udiskie. Ripristina con strata --uninstall-udiskie-unlock.'
    [custom_title]='Configura Strata a modo tuo'
    [plan]='Piano di installazione'
    [destination]='Installa in %{path}'
    [defaults_unchanged]='Le applicazioni predefinite e le scorciatoie resteranno invariate.'
    [dependencies_ready]='Tutti i pacchetti di esecuzione selezionati sono già installati.'
    [dependencies]='Installa le dipendenze'
    [packages]='Pacchetti da installare: %{packages}'
    [reinstall]='%{path} verrà sostituito. Le impostazioni salvate saranno conservate.'
    [installing]='Installazione di Strata'
    [download]='Scarica Strata %{version}'
    [download_manifest]='Scarica il manifesto firmato della versione'
    [download_signatures]='Scarica le firme della versione'
    [signature]='Verifica della firma della versione'
    [digest]='Verifica del digest SHA-256 firmato'
    [unpack]='Estrai la versione verificata'
    [application]='Installa l’applicazione'
    [ready]='Pronto per esplorare'
    [installed_notices]='Installato — controlla i dettagli della configurazione'
    [installed_warnings]='Installazione completata con avvisi'
    [installed]='Strata %{version} è installato.'
    [launch]='Esegui %{command} per iniziare.'
    [usage_intro]='Senza opzioni, scegli l’installazione consigliata o personalizza l’integrazione desktop.'
    [verbose_help]='Mostra i dettagli di download e verifica'
    [udiskie_requires]='--with-udiskie-unlock richiede un sistema basato su Arch con udiskie nel PATH.'
    [progress]='%{percent}%'
  ) ;;
  ru) INSTALLER_MESSAGES=(
    [setup]='Интерактивная установка'
    [tagline]='Перемещайтесь по всем уровням.'
    [partial]='Strata установлена, но настройка не завершена.'
    [failed]='Установка не завершена.'
    [details]='Подробный журнал: %{path}'
    [cancelled]='Установка отменена. Изменений нет.'
    [task_failed]='Ошибка: %{task}'
    [read_failed]='Не удалось прочитать ваш выбор.'
    [keys_toggle]='↑/↓ переход · Пробел выбор · Enter установка · Esc отмена'
    [keys_choose]='↑/↓ выбор · Enter продолжить · Esc отмена'
    [plain_toggle]='Выбор по номеру; Enter для установки, q для отмены:'
    [plain_choose]='Выберите [1] или q для отмены:'
    [invalid_selection]='Выберите один из предложенных вариантов.'
    [standard]='Установить Strata (рекомендуется)'
    [custom]='Настроить установку'
    [standard_hint]='Всё, что поддерживает эта система: ярлык запуска, папки, диалог выбора файлов и дополнительные интеграции.'
    [custom_hint]='Выберите интеграцию с рабочим столом и дополнительные возможности.'
    [setup_title]='Выберите вариант установки'
    [launcher]='Ярлык запуска'
    [folders]='Файловый менеджер по умолчанию'
    [reveal]='Открытие расположения файла'
    [chooser]='Диалоги открытия и сохранения'
    [launcher_hint]='Добавить Strata в меню приложений.'
    [folders_hint]='Открывать папки и расположение файлов в Strata. Включает ярлык запуска.'
    [reveal_hint]='Показывать файлы из других приложений, не меняя обработчик папок.'
    [reveal_ready]='Теперь Strata обрабатывает запросы «Открытие расположения файла».'
    [reveal_relogin]='Служба FileManager1 установлена, но перезагрузить сеансовую шину не удалось. Выйдите из системы и войдите снова, чтобы Strata обрабатывала запросы «Открытие расположения файла».'
    [chooser_hint]='Использовать Strata для диалогов портала. Служба будет перезапущена; сначала закройте файловые диалоги.'
    [smb]='Сетевые ресурсы Windows'
    [smb_hint]='Установить поддержку SMB для просмотра сетевых ресурсов.'
    [raw]='Поддержка фотографий RAW'
    [raw_hint]='Установить дополнительные декодеры для предпросмотра изображений.'
    [omarchy]='Горячие клавиши файлового менеджера Omarchy'
    [omarchy_hint]='Заменить горячие клавиши файлового менеджера. Резервная копия сохранится.'
    [udiskie]='Разблокировка зашифрованных дисков'
    [udiskie_hint]='Изменить настройки udiskie, отключить автомонтирование LUKS и перезапустить udiskie. Восстановление: strata --uninstall-udiskie-unlock.'
    [custom_title]='Настройте Strata под себя'
    [plan]='План установки'
    [destination]='Установить в %{path}'
    [defaults_unchanged]='Приложения по умолчанию и горячие клавиши не изменятся.'
    [dependencies_ready]='Все выбранные пакеты для работы уже установлены.'
    [dependencies]='Установка зависимостей'
    [packages]='Пакеты для установки: %{packages}'
    [reinstall]='%{path} будет заменён. Сохранённые настройки останутся.'
    [installing]='Установка Strata'
    [download]='Загрузка Strata %{version}'
    [download_manifest]='Загрузка подписанного манифеста выпуска'
    [download_signatures]='Загрузка подписей выпуска'
    [signature]='Проверка подписи выпуска'
    [digest]='Проверка подписанной контрольной суммы SHA-256'
    [unpack]='Распаковка проверенного выпуска'
    [application]='Установка приложения'
    [ready]='Всё готово к работе'
    [installed_notices]='Установлено — проверьте сведения о настройке'
    [installed_warnings]='Установлено с предупреждениями'
    [installed]='Strata %{version} установлена.'
    [launch]='Для начала работы выполните %{command}.'
    [usage_intro]='Без параметров выберите рекомендуемую установку или настройте интеграцию с рабочим столом.'
    [verbose_help]='Показать подробности загрузки и проверки'
    [udiskie_requires]='--with-udiskie-unlock требует систему на базе Arch с udiskie в PATH.'
    [progress]='%{percent} %'
  ) ;;
  zh-CN) INSTALLER_MESSAGES=(
    [setup]='交互式安装程序'
    [tagline]='穿梭每一层文件夹。'
    [partial]='Strata 已安装，但设置尚未完成。'
    [failed]='安装尚未完成。'
    [details]='详细日志：%{path}'
    [cancelled]='已取消安装，未做任何更改。'
    [task_failed]='失败：%{task}'
    [read_failed]='无法读取所选项。'
    [keys_toggle]='↑/↓ 移动 · 空格切换 · Enter 安装 · Esc 取消'
    [keys_choose]='↑/↓ 选择 · Enter 继续 · Esc 取消'
    [plain_toggle]='输入编号切换选项，按 Enter 安装，输入 q 取消：'
    [plain_choose]='选择 [1]，或输入 q 取消：'
    [invalid_selection]='请选择列表中的选项。'
    [standard]='安装 Strata（推荐）'
    [custom]='自定义安装'
    [standard_hint]='此系统支持的全部内容：启动器、文件夹、文件选择器和可选集成。'
    [custom_hint]='选择桌面集成和可选功能。'
    [setup_title]='选择安装方式'
    [launcher]='应用启动图标'
    [folders]='默认文件管理器'
    [reveal]='打开文件位置'
    [chooser]='打开和保存对话框'
    [launcher_hint]='将 Strata 添加到应用菜单。'
    [folders_hint]='使用 Strata 打开文件夹和文件位置，并添加启动图标。'
    [reveal_hint]='显示其他应用中的文件位置，不更改默认文件夹处理程序。'
    [reveal_ready]='Strata 现在会处理“打开文件位置”请求。'
    [reveal_relogin]='已安装 FileManager1 服务，但无法重新加载会话总线。请注销并重新登录，以便使用 Strata 处理“打开文件位置”请求。'
    [chooser_hint]='使用 Strata 的门户对话框。此操作会重启门户服务，请先关闭文件对话框。'
    [smb]='Windows 网络共享'
    [smb_hint]='安装可选的 SMB 支持，以浏览网络共享。'
    [raw]='相机 RAW 格式支持'
    [raw_hint]='安装可选的图像解码器，以预览更多格式。'
    [omarchy]='Omarchy 文件管理器快捷键'
    [omarchy_hint]='替换文件管理器快捷键，并保留备份。'
    [udiskie]='加密驱动器解锁'
    [udiskie_hint]='修改 udiskie 设置、关闭 LUKS 自动挂载并重启 udiskie。可运行 strata --uninstall-udiskie-unlock 恢复。'
    [custom_title]='按你的需要设置 Strata'
    [plan]='安装计划'
    [destination]='安装到 %{path}'
    [defaults_unchanged]='保留当前默认应用和快捷键。'
    [dependencies_ready]='所选运行时软件包均已安装。'
    [dependencies]='安装依赖项'
    [packages]='将安装的软件包：%{packages}'
    [reinstall]='将替换 %{path}，保留已保存的设置。'
    [installing]='正在安装 Strata'
    [download]='下载 Strata %{version}'
    [download_manifest]='下载已签名的发行版清单'
    [download_signatures]='下载发行版签名'
    [signature]='验证发行版签名'
    [digest]='验证已签名的 SHA-256 摘要'
    [unpack]='解压已验证的发行版'
    [application]='安装应用'
    [ready]='开始探索吧'
    [installed_notices]='已安装，请查看设置详情'
    [installed_warnings]='已安装，但有警告'
    [installed]='已安装 Strata %{version}。'
    [launch]='运行 %{command} 开始使用。'
    [usage_intro]='不带选项运行时，可选择推荐安装或自定义桌面集成。'
    [verbose_help]='显示下载和验证的详细输出'
    [udiskie_requires]='--with-udiskie-unlock 需要基于 Arch 的系统，且 PATH 中包含 udiskie。'
    [progress]='%{percent}%'
  ) ;;
  zh-TW) INSTALLER_MESSAGES=(
    [setup]='互動式安裝程式'
    [tagline]='穿梭每一層資料夾。'
    [partial]='Strata 已安裝，但設定尚未完成。'
    [failed]='安裝尚未完成。'
    [details]='詳細輸出：%{path}'
    [cancelled]='已取消安裝，未做任何變更。'
    [task_failed]='失敗：%{task}'
    [read_failed]='無法讀取你的選擇。'
    [keys_toggle]='↑/↓ 移動 · 空白鍵切換 · Enter 安裝 · Esc 取消'
    [keys_choose]='↑/↓ 選擇 · Enter 繼續 · Esc 取消'
    [plain_toggle]='輸入編號切換選項，按 Enter 安裝，輸入 q 取消：'
    [plain_choose]='選擇 [1]，或輸入 q 取消：'
    [invalid_selection]='請選擇列出的其中一個選項。'
    [standard]='安裝 Strata（建議）'
    [custom]='自訂安裝'
    [standard_hint]='此系統支援的全部內容：啟動器、資料夾、檔案選擇器和選用整合。'
    [custom_hint]='選擇桌面整合與選用支援。'
    [setup_title]='選擇安裝方式'
    [launcher]='應用程式啟動器'
    [folders]='預設檔案管理員'
    [reveal]='開啟檔案位置'
    [chooser]='開啟與儲存對話框'
    [launcher_hint]='將 Strata 加入應用程式選單。'
    [folders_hint]='使用 Strata 開啟資料夾與檔案位置，並包含啟動器。'
    [reveal_hint]='從其他應用程式顯示檔案位置，不變更資料夾的預設應用程式。'
    [reveal_ready]='Strata 現在會處理“開啟檔案位置”要求。'
    [reveal_relogin]='已安裝 FileManager1 服務，但無法重新載入工作階段匯流排。請登出並重新登入，以便使用 Strata 處理“開啟檔案位置”要求。'
    [chooser_hint]='使用 Strata 作為入口對話框。這會重新啟動入口服務，請先關閉檔案對話框。'
    [smb]='Windows 網路共享'
    [smb_hint]='安裝選用的 SMB 支援，以瀏覽網路共享。'
    [raw]='相機 RAW 支援'
    [raw_hint]='安裝選用的影像解碼器，以預覽更多格式。'
    [omarchy]='Omarchy 檔案管理員快捷鍵'
    [omarchy_hint]='取代檔案管理員快捷鍵，並保留備份。'
    [udiskie]='加密磁碟機解鎖'
    [udiskie_hint]='編輯 udiskie 設定、停用 LUKS 自動掛載並重新啟動 udiskie。可執行 strata --uninstall-udiskie-unlock 還原。'
    [custom_title]='打造你的 Strata'
    [plan]='安裝計畫'
    [destination]='安裝到 %{path}'
    [defaults_unchanged]='你的預設應用程式與快捷鍵將保持不變。'
    [dependencies_ready]='所有選取的執行階段套件皆已安裝。'
    [dependencies]='安裝相依套件'
    [packages]='要安裝的套件：%{packages}'
    [reinstall]='這會取代 %{path}，並保留你已儲存的設定。'
    [installing]='正在安裝 Strata'
    [download]='下載 Strata %{version}'
    [download_manifest]='下載已簽署的發行版本清單'
    [download_signatures]='下載發行版本簽章'
    [signature]='驗證發行版本簽章'
    [digest]='驗證已簽署的 SHA-256 摘要'
    [unpack]='解壓縮已驗證的發行版本'
    [application]='安裝應用程式'
    [ready]='開始探索吧'
    [installed_notices]='已安裝，請查看設定詳細資訊'
    [installed_warnings]='已安裝，但有警告'
    [installed]='已安裝 Strata %{version}。'
    [launch]='執行 %{command} 開始使用。'
    [usage_intro]='不帶選項執行時，可選擇建議的安裝方式或自訂桌面整合。'
    [verbose_help]='顯示詳細的下載與驗證輸出'
    [udiskie_requires]='--with-udiskie-unlock 需要以 Arch 為基礎、且 PATH 中有 udiskie 的系統。'
    [progress]='%{percent}%'
  ) ;;
esac
# installer-translations:end

text() {
  local message=${INSTALLER_MESSAGES[$1]:-$1} argument name value
  shift
  for argument in "$@"; do
    name=${argument%%=*}
    value=${argument#*=}
    message=${message//"%{$name}"/"$value"}
  done
  printf '%s' "$message"
}

setup_terminal() {
  local rows columns
  if [[ -t 1 && ${TERM:-dumb} != dumb ]]; then
    if [[ -t 2 && -z ${NO_COLOR:-} ]]; then
      COLOR_RESET=$'\033[0m' COLOR_BOLD=$'\033[1m' COLOR_DIM=$'\033[2m'
      COLOR_ACCENT=$'\033[36m' COLOR_SUCCESS=$'\033[32m'
      COLOR_WARNING=$'\033[33m' COLOR_ERROR=$'\033[31m'
      COLOR_LAYER_ONE=$'\033[34m' COLOR_LAYER_TWO=$'\033[35m'
    fi
    if [[ -n ${PROMPT_DEVICE:-} ]] && command -v stty >/dev/null 2>&1 \
      && read -r rows columns < <(stty size <"$PROMPT_DEVICE" 2>/dev/null) \
      && ((rows >= 24 && columns >= 64)); then
      RICH_TERMINAL=yes
    fi
  fi
}

info() {
  printf '  %s✓%s  %s\n' "$COLOR_SUCCESS" "$COLOR_RESET" "$*"
}

note() {
  printf '  %s!%s  %s\n' "$COLOR_WARNING" "$COLOR_RESET" "$*" >&2
}

warn() {
  WARNING_COUNT=$((WARNING_COUNT + 1))
  note "$@"
  [[ -z $LOG_FILE ]] || printf 'warning: %s\n' "$*" >>"$LOG_FILE"
}

die() {
  printf '  %s×%s  %s\n' "$COLOR_ERROR" "$COLOR_RESET" "$*" >&2
  [[ -z $LOG_FILE ]] || printf 'error: %s\n' "$*" >>"$LOG_FILE"
  exit 1
}

section() {
  printf '\n  %s%s%s%s\n\n' "$COLOR_ACCENT" "$COLOR_BOLD" "$*" "$COLOR_RESET"
}

divider() {
  if [[ $RICH_TERMINAL == yes ]]; then
    printf '  %s────────────────────────────────────────────────────────────%s\n' "$COLOR_DIM" "$COLOR_RESET"
  fi
}

installer_cleanup() {
  local status=$?
  [[ $RICH_TERMINAL != yes ]] || printf '\033[?25h'
  if [[ -n $ACTIVE_PID ]]; then
    kill "$ACTIVE_PID" 2>/dev/null || true
    wait "$ACTIVE_PID" 2>/dev/null || true
  fi
  [[ -z ${TEMP_DIR:-} ]] || rm -rf -- "$TEMP_DIR"
  if ((status != 0)) && [[ -n $LOG_FILE ]]; then
    if [[ $CORE_INSTALLED == yes ]]; then
      warn "$(text partial)"
    else
      warn "$(text failed)"
    fi
    printf '  %s\n' "$(text details "path=$LOG_FILE")" >&2
  elif [[ -n $LOG_FILE && $WARNING_COUNT == 0 && $SETUP_NOTICES == no ]]; then
    rm -f -- "$LOG_FILE"
  fi
}

cancel_installation() {
  [[ $RICH_TERMINAL != yes ]] || printf '\0338\033[J\033[?25h'
  printf '\n  %s\n\n' "$(text cancelled)"
  exit 0
}

# Reads curl's own progress meter, which has no percentage while the total is unknown.
download_percent() {
  local progress
  progress=$(tail -c 2048 "$TEMP_DIR/task-output" 2>/dev/null) || return 1
  progress=${progress%$'\r'}
  progress=${progress##*$'\r'}
  [[ $progress =~ ([0-9]{1,3})[.,][0-9]+% ]] || return 1
  local percent=$((10#${BASH_REMATCH[1]}))
  ((percent <= 100)) || return 1
  printf '%s' "$percent"
}

progress_bar() {
  local percent=$1 marker=$2 ribbon="" color cell i
  local filled=$((percent * 27 / 100))
  local -a layers=("$COLOR_LAYER_ONE" "$COLOR_LAYER_TWO" "$COLOR_ACCENT")
  for ((i = 0; i < 27; i++)); do
    if ((i < filled)); then
      color=${layers[i / 9]} cell='▓'
      [[ $marker != '✓' ]] || color=$COLOR_SUCCESS
    elif ((i == filled && percent > 0)); then
      color=$COLOR_ACCENT$COLOR_BOLD cell='▒'
    else
      color=$COLOR_DIM cell='░'
    fi
    ribbon+=$COLOR_RESET$color$cell
  done
  color=$COLOR_ACCENT
  [[ $marker != '✓' ]] || color=$COLOR_SUCCESS
  printf '\r  %s%s%s  %s%s  %s%s%s\033[K' "$color" "$marker" "$COLOR_RESET" \
    "$ribbon" "$COLOR_RESET" "$color" "$(text progress "percent=$percent")" "$COLOR_RESET"
}

run_task() {
  local label=$1 result frame=0 percent output_mode=${TASK_OUTPUT:-quiet}
  local -a frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
  shift
  if [[ -z $LOG_FILE ]]; then
    "$@"
    return
  fi
  printf '\n--- %s ---\n' "$label" >>"$LOG_FILE"
  : >"$TEMP_DIR/task-output"
  if [[ $VERBOSE == yes || $output_mode == live ]]; then
    printf '  %s…%s  %s\n' "$COLOR_ACCENT" "$COLOR_RESET" "$label"
    if "$@" 2>&1 | tee -a "$LOG_FILE" "$TEMP_DIR/task-output"; then result=0; else result=$?; fi
  else
    "$@" >"$TEMP_DIR/task-output" 2>&1 &
    ACTIVE_PID=$!
    if [[ $RICH_TERMINAL != yes ]]; then
      printf '  …  %s\n' "$label"
    elif [[ ${TASK_PROGRESS:-no} == yes ]]; then
      printf '  %s%s%s\n' "$COLOR_BOLD" "$label" "$COLOR_RESET"
    fi
    while kill -0 "$ACTIVE_PID" 2>/dev/null; do
      if [[ $RICH_TERMINAL == yes ]]; then
        if [[ ${TASK_PROGRESS:-no} == yes ]] && percent=$(download_percent); then
          progress_bar "$percent" "${frames[frame % 10]}"
        elif [[ ${TASK_PROGRESS:-no} == yes ]]; then
          printf '\r  %s%s%s\033[K' "$COLOR_ACCENT" "${frames[frame % 10]}" "$COLOR_RESET"
        else
          printf '\r  %s%s%s  %s\033[K' "$COLOR_ACCENT" "${frames[frame % 10]}" "$COLOR_RESET" "$label"
        fi
        frame=$((frame + 1))
      fi
      sleep 0.1
    done
    if wait "$ACTIVE_PID"; then result=0; else result=$?; fi
    ACTIVE_PID=""
    [[ $RICH_TERMINAL != yes ]] || printf '\r\033[K'
    cat "$TEMP_DIR/task-output" >>"$LOG_FILE"
    if ((result != 0)); then
      cat "$TEMP_DIR/task-output" >&2
    fi
  fi
  if ((result != 0)); then
    printf '  %s×%s  %s\n' "$COLOR_ERROR" "$COLOR_RESET" "$(text task_failed "task=$label")" >&2
    return "$result"
  fi
  # Integration commands can succeed while reporting service-restart warnings.
  if [[ $output_mode == notices && -s $TEMP_DIR/task-output ]]; then
    SETUP_NOTICES=yes
    [[ $VERBOSE == yes ]] || cat "$TEMP_DIR/task-output"
  fi
  if [[ $RICH_TERMINAL == yes && $VERBOSE == no && ${TASK_PROGRESS:-no} == yes ]]; then
    progress_bar 100 '✓'
    printf '\n'
  else
    info "$label"
  fi
}

hex_to_bytes() {
  # shellcheck disable=SC2059
  printf "$(sed 's/../\\x&/g' <<<"$1")"
}

file_size() {
  printf '%d' "$(wc -c <"$1")"
}

download_release_file() {
  local label=$1 url=$2 output=$3 limit=$4

  run_task "$label" curl --fail --location --show-error --silent --max-filesize "$limit" \
    --output "$output" "$url"
  (($(file_size "$output") <= limit)) \
    || die "${output##*/} is larger than $limit bytes; refusing to continue."
}

# The signatures file is the canonical compact JSON written by scripts/sign_update_manifest.py.
verify_release_signature() {
  local manifest=$1 signatures=$2 key key_id entry signature
  local -a entries

  mapfile -t entries < <(grep -oE '\{"key_id":"[0-9a-f]{64}","signature":"[0-9a-f]{128}"\}' "$signatures")
  { printf 'strata-update-manifest-v1\0'; cat "$manifest"; } >"$TEMP_DIR/signed-message"
  for key in "${RELEASE_PUBLIC_KEYS[@]}"; do
    key_id=$(hex_to_bytes "$key" | sha256sum | sed 's/ .*//')
    hex_to_bytes "302a300506032b6570032100$key" >"$TEMP_DIR/release-key.der"
    for entry in "${entries[@]}"; do
      [[ $entry == "{\"key_id\":\"$key_id\","* ]] || continue
      signature=${entry#*\"signature\":\"}
      hex_to_bytes "${signature%\"\}}" >"$TEMP_DIR/release-signature"
      if openssl pkeyutl -verify -rawin -pubin -keyform DER -inkey "$TEMP_DIR/release-key.der" \
          -sigfile "$TEMP_DIR/release-signature" -in "$TEMP_DIR/signed-message"; then
        return 0
      fi
    done
  done
  echo "No trusted release key signed $MANIFEST_NAME." >&2
  return 1
}

read_signed_artifact() {
  local manifest=$1 version=$2 target=$3 archive=$4 content pattern

  content=$(<"$manifest")
  pattern="^\\{\"artifacts\":\\[.*\\],\"repository\":\"$REPOSITORY\",\"schema\":1,"
  pattern+="\"source_commit\":\"[0-9a-f]{40}\",\"tag\":\"v${version//./\\.}\"\\}$"
  [[ $content =~ $pattern ]] \
    || die "The signed manifest does not describe Strata $version from $REPOSITORY."
  pattern="\\{\"name\":\"${archive//./\\.}\",\"sha256\":\"([0-9a-f]{64})\",\"size\":([1-9][0-9]*),"
  pattern+="\"target\":\"$target\"\\}"
  [[ $content =~ $pattern ]] \
    || die "The signed manifest does not list $archive for $target."
  SIGNED_ARCHIVE_SHA256=${BASH_REMATCH[1]}
  SIGNED_ARCHIVE_SIZE=${BASH_REMATCH[2]}
  ((SIGNED_ARCHIVE_SIZE <= MAX_ARCHIVE_BYTES)) \
    || die "The signed manifest lists an archive larger than $MAX_ARCHIVE_BYTES bytes."
}

verify_archive_digest() {
  local archive=$1 size digest

  size=$(file_size "$archive")
  if ((size != SIGNED_ARCHIVE_SIZE)); then
    echo "${archive##*/} is $size bytes; the signed manifest expects $SIGNED_ARCHIVE_SIZE." >&2
    return 1
  fi
  digest=$(sha256sum "$archive" | sed 's/ .*//')
  if [[ $digest != "$SIGNED_ARCHIVE_SHA256" ]]; then
    echo "${archive##*/} has SHA-256 $digest; the signed manifest expects $SIGNED_ARCHIVE_SHA256." >&2
    return 1
  fi
  echo "${archive##*/}: OK"
}

show_banner() {
  local reset="" bold=""
  local -a main_colors=("" "" "" "" "" "" "" "" "" "")
  local -a top_colors=("" "" "") bottom_colors=("" "" "")

  if [[ -t 1 && ${TERM:-dumb} != dumb && -z ${NO_COLOR:-} ]]; then
    reset=$'\033[0m'
    bold=$'\033[1m'
    main_colors=(
      $'\033[38;2;199;210;255m'
      $'\033[38;2;193;205;252m'
      $'\033[38;2;187;200;250m'
      $'\033[38;2;181;196;247m'
      $'\033[38;2;175;191;245m'
      $'\033[38;2;169;186;242m'
      $'\033[38;2;163;181;240m'
      $'\033[38;2;157;177;237m'
      $'\033[38;2;151;172;235m'
      $'\033[38;2;145;167;232m'
    )
    top_colors=($'\033[38;2;156;203;255m' $'\033[38;2;127;185;255m' $'\033[38;2;98;168;255m')
    bottom_colors=($'\033[38;2;157;140;255m' $'\033[38;2;130;128;255m' $'\033[38;2;102;116;255m')
  fi

  printf '\n'
  printf '%b%s%b\n' "${main_colors[0]}" '         ▄▄██▄' "$reset"
  printf '%b%s%b%s%b         %bS T R A T A%b\n' "${main_colors[1]}" '      ▄████▀▀   ' "${top_colors[0]}" '▄▄▄' "$reset" "$bold" "$reset"
  printf '%b%s%b%s%b       %s\n' "${main_colors[2]}" '   ▄████▀      ' "${top_colors[1]}" '▀▀███▄' "$reset" "$(text tagline)"
  printf '%b%s%b%s%b\n' "${main_colors[3]}" '   ███    ████▄▄   ' "${top_colors[2]}" '▀▀' "$reset"
  printf '%b%s%b\n' "${main_colors[4]}" '   ███▄▄    ▀▀███▄▄' "$reset"
  printf '%b%s%b\n' "${main_colors[5]}" '    ▀▀███▄▄    ▀▀████' "$reset"
  printf '%b%s%b%s%b\n' "${bottom_colors[0]}" '   ▄' "${main_colors[6]}" '   ▀▀████▄    ███' "$reset"
  printf '%b%s%b%s%b\n' "${bottom_colors[1]}" '   ███▄▄' "${main_colors[7]}" '   ▀▀   ▄████' "$reset"
  printf '%b%s%b%s%b         %s\n' "${bottom_colors[2]}" '    ▀▀█▀▀' "${main_colors[8]}" '   ▄▄███▀▀' "$reset" "$(text setup)"
  printf '%b%s%b\n\n' "${main_colors[9]}" '          ████▀▀' "$reset"
}

read_menu_key() {
  local continuation
  MENU_KEY=""
  IFS= read -rsn1 MENU_KEY <"$PROMPT_DEVICE" || die "$(text read_failed)"
  if [[ $MENU_KEY == $'\033' ]]; then
    if IFS= read -rsn1 -t 0.1 continuation <"$PROMPT_DEVICE"; then
      MENU_KEY+=$continuation
      if [[ $continuation == '[' || $continuation == O ]]; then
        IFS= read -rsn1 -t 0.1 continuation <"$PROMPT_DEVICE" || true
        MENU_KEY+=$continuation
      fi
    fi
  fi
}

select_menu() {
  local title=$1 multiple=$2 labels_name=$3 hints_name=$4 choices_name=$5 changed=${6:-:}
  local -n menu_labels=$labels_name menu_hints=$hints_name menu_choices=$choices_name
  local active=0 i answer marker color
  if [[ $RICH_TERMINAL == yes ]]; then
    local menu_rows=$((${#menu_labels[@]} + 12))
    # Reserve space before saving the cursor so redraws cannot scroll its anchor.
    for ((i = 0; i < menu_rows; i++)); do printf '\n'; done
    printf '\033[%dA\0337\033[?25l' "$menu_rows"
  fi
  while true; do
    [[ $RICH_TERMINAL != yes ]] || printf '\0338\033[J'
    section "$title"
    for i in "${!menu_labels[@]}"; do
      marker=' ' color=$COLOR_RESET
      if [[ $RICH_TERMINAL != yes ]]; then
        marker=$((i + 1))
      elif ((i == active)); then
        marker='›' color=$COLOR_ACCENT$COLOR_BOLD
      elif [[ $multiple == yes && ${menu_choices[i]} == yes ]]; then
        color=$COLOR_SUCCESS
      fi
      if [[ $multiple == yes ]]; then
        local check=' '
        [[ ${menu_choices[i]} != yes ]] || check='✓'
        printf '  %s%s  [%s] %s%s\n' "$color" "$marker" "$check" "${menu_labels[i]}" "$COLOR_RESET"
      else
        printf '  %s%s  %s%s\n' "$color" "$marker" "${menu_labels[i]}" "$COLOR_RESET"
      fi
    done
    if [[ $RICH_TERMINAL == yes ]]; then
      printf '\n  %s%s%s\n\n' "$COLOR_DIM" "${menu_hints[active]}" "$COLOR_RESET"
      divider
      if [[ $multiple == yes ]]; then
        printf '  %s%s%s\n' "$COLOR_DIM" "$(text keys_toggle)" "$COLOR_RESET"
      else
        printf '  %s%s%s\n' "$COLOR_DIM" "$(text keys_choose)" "$COLOR_RESET"
      fi
      read_menu_key
      case $MENU_KEY in
        $'\033[A' | $'\033OA' | k) active=$(((active + ${#menu_labels[@]} - 1) % ${#menu_labels[@]})) ;;
        $'\033[B' | $'\033OB' | j) active=$(((active + 1) % ${#menu_labels[@]})) ;;
        ' ')
          if [[ $multiple == yes ]]; then
            if [[ ${menu_choices[active]} == yes ]]; then menu_choices[active]=no; else menu_choices[active]=yes; fi
            "$changed" "$choices_name" "$active"
          fi
          ;;
        '')
          [[ $multiple == yes ]] || menu_choices[active]=yes
          printf '\0338\033[J\033[?25h'
          return
          ;;
        $'\033' | q | Q) cancel_installation ;;
      esac
    else
      printf '\n'
      for i in "${!menu_labels[@]}"; do
        printf '  %s: %s\n' "${menu_labels[i]}" "${menu_hints[i]}"
      done
      if [[ $multiple == yes ]]; then
        printf '\n  %s ' "$(text plain_toggle)" >"$PROMPT_DEVICE"
      else
        printf '\n  %s ' "$(text plain_choose)" >"$PROMPT_DEVICE"
      fi
      IFS= read -r answer <"$PROMPT_DEVICE" || die "$(text read_failed)"
      case $answer in
        q | Q) cancel_installation ;;
        '')
          [[ $multiple == yes ]] || menu_choices[0]=yes
          return
          ;;
        *)
          if [[ $answer =~ ^[1-9][0-9]*$ ]] && ((${#answer} <= 2 && 10#$answer <= ${#menu_labels[@]})); then
            active=$((10#$answer - 1))
            if [[ $multiple == no ]]; then
              menu_choices[active]=yes
              return
            fi
            if [[ ${menu_choices[active]} == yes ]]; then menu_choices[active]=no; else menu_choices[active]=yes; fi
            "$changed" "$choices_name" "$active"
          else
            note "$(text invalid_selection)"
          fi
          ;;
      esac
    fi
  done
}

packages_installed() {
  local package
  for package in "$@"; do
    pacman -Q "$package" >/dev/null 2>&1 || return 1
  done
}

integration_dependencies() {
  local -n choices=$1
  local changed=$2 i folder=-1 launcher=-1 reveal=-1
  for i in "${!option_flags[@]}"; do
    case ${option_flags[i]} in
      WITH_FOLDER_ASSOCIATION) folder=$i ;;
      WITH_DESKTOP_ENTRY) launcher=$i ;;
      WITH_FILE_MANAGER) reveal=$i ;;
    esac
  done
  if ((folder >= 0 && (changed == launcher || changed == reveal))) && [[ ${choices[changed]} == no ]]; then
    choices[folder]=no
  elif ((folder >= 0)) && [[ ${choices[folder]} == yes ]]; then
    choices[launcher]=yes
    choices[reveal]=yes
  fi
}

# Unspecified integrations default to everything this system supports. Explicit
# --with/--without flags and the custom menu override that.
resolve_default_integrations() {
  local arch_based=$1 omarchy_major=$2 flag
  local -a supported=(WITH_DESKTOP_ENTRY WITH_FOLDER_ASSOCIATION WITH_FILE_MANAGER WITH_FILE_CHOOSER)
  [[ $arch_based != yes ]] || supported+=(WITH_SMB WITH_RAW)
  [[ -z $omarchy_major ]] || supported+=(WITH_OMARCHY_KEYBINDS)
  if [[ $arch_based == yes ]] && udiskie_on_path; then supported+=(WITH_UDISKIE_UNLOCK); fi
  for flag in "${supported[@]}"; do
    [[ ${!flag} != ask ]] || printf -v "$flag" '%s' yes
  done
}

choose_installation() {
  local arch_based=$1 omarchy_major=$2 i
  local -a setup_labels=("$(text standard)" "$(text custom)")
  local -a setup_hints=("$(text standard_hint)" "$(text custom_hint)")
  local -a setup_choices=(no no)
  [[ $NON_INTERACTIVE == no ]] || return 0
  select_menu "$(text setup_title)" no setup_labels setup_hints setup_choices >"$PROMPT_DEVICE"

  local -a option_flags=(WITH_DESKTOP_ENTRY WITH_FOLDER_ASSOCIATION WITH_FILE_MANAGER WITH_FILE_CHOOSER)
  local -a option_labels=("$(text launcher)" "$(text folders)" "$(text reveal)" "$(text chooser)")
  local -a option_hints=("$(text launcher_hint)" "$(text folders_hint)" "$(text reveal_hint)" "$(text chooser_hint)")
  if [[ $arch_based == yes ]]; then
    if ! packages_installed gvfs-smb; then
      option_flags+=(WITH_SMB)
      option_labels+=("$(text smb)")
      option_hints+=("$(text smb_hint)")
    fi
    if ! packages_installed "${RAW_PREVIEW_PACKAGES[@]}"; then
      option_flags+=(WITH_RAW)
      option_labels+=("$(text raw)")
      option_hints+=("$(text raw_hint)")
    fi
  fi
  if [[ -n $omarchy_major ]]; then
    option_flags+=(WITH_OMARCHY_KEYBINDS)
    option_labels+=("$(text omarchy)")
    option_hints+=("$(text omarchy_hint)")
  fi
  if [[ $arch_based == yes || -n $omarchy_major ]] && udiskie_on_path; then
    option_flags+=(WITH_UDISKIE_UNLOCK)
    option_labels+=("$(text udiskie)")
    option_hints+=("$(text udiskie_hint)")
  fi
  local -a option_choices=()
  for i in "${option_flags[@]}"; do
    if [[ ${!i} == yes ]]; then option_choices+=(yes); else option_choices+=(no); fi
  done
  if [[ ${setup_choices[1]} == yes ]]; then
    select_menu "$(text custom_title)" yes option_labels option_hints option_choices integration_dependencies >"$PROMPT_DEVICE"
  fi
  for i in WITH_SMB WITH_RAW WITH_DESKTOP_ENTRY WITH_FOLDER_ASSOCIATION WITH_FILE_MANAGER \
    WITH_FILE_CHOOSER WITH_OMARCHY_KEYBINDS WITH_UDISKIE_UNLOCK; do
    printf -v "$i" '%s' no
  done
  for i in "${!option_flags[@]}"; do
    printf -v "${option_flags[i]}" '%s' "${option_choices[i]}"
  done
}

show_installation_plan() {
  local i
  section "$(text plan)"
  printf '  %s\n' "$(text destination "path=$BIN_PATH")"
  local -a flags=(WITH_DESKTOP_ENTRY WITH_FOLDER_ASSOCIATION WITH_FILE_MANAGER WITH_FILE_CHOOSER WITH_SMB WITH_RAW WITH_OMARCHY_KEYBINDS WITH_UDISKIE_UNLOCK)
  local -a labels=(launcher folders reveal chooser smb raw omarchy udiskie)
  for i in "${!flags[@]}"; do
    [[ ${!flags[i]} != yes ]] || printf '  %s+%s  %s\n' "$COLOR_ACCENT" "$COLOR_RESET" "$(text "${labels[i]}")"
  done
  if [[ $WITH_FOLDER_ASSOCIATION != yes && $WITH_FILE_MANAGER != yes && $WITH_FILE_CHOOSER != yes \
    && $WITH_OMARCHY_KEYBINDS != yes && $WITH_UDISKIE_UNLOCK != yes ]]; then
    printf '\n  %s%s%s\n' "$COLOR_DIM" "$(text defaults_unchanged)" "$COLOR_RESET"
  fi
}

prompt() {
  local question=$1 default=${2:-yes} answer suffix
  if [[ $default == yes ]]; then
    suffix="[Y/n]"
  else
    suffix="[y/N]"
  fi

  printf '%s %s ' "$question" "$suffix" >"$PROMPT_DEVICE"
  IFS= read -r answer <"$PROMPT_DEVICE" || die "Could not read your answer."
  answer=${answer:-$default}
  [[ $answer == [Yy] || $answer == [Yy][Ee][Ss] ]]
}

usage() {
  cat <<EOF
Usage: install.sh [options]

$(text usage_intro)

Options:
  --non-interactive             Never prompt; install every supported integration
  --verbose                     $(text verbose_help)
  --with-smb                    Install SMB network-share support
  --with-raw                    Install broader image and camera RAW support
  --with-desktop-entry          Add Strata to the desktop application menu
  --with-folder-association     Make Strata the default folder handler
  --with-file-manager           Handle "Open file location" requests
  --with-file-chooser           Use Strata for portal Open and Save dialogs
  --with-omarchy-keybinds       Replace Omarchy's file-manager keybinds
  --with-udiskie-unlock         Use Strata for udiskie encrypted-volume unlock
  --without-smb                 Skip SMB network-share support
  --without-raw                 Skip broader image and camera RAW support
  --without-desktop-entry       Skip the application menu entry and folder handler
  --without-folder-association  Keep the current default folder handler
  --without-file-manager        Skip "Open file location" and folder handling
  --without-file-chooser        Keep the current chooser; suppress the app offer
  --without-omarchy-keybinds    Keep Omarchy's current file-manager keybinds
  --without-udiskie-unlock      Keep the current udiskie unlock setup
  -h, --help                    Show this help

Integration flags imply --non-interactive. Every integration this system supports
is installed unless a --without-* flag declines it; a --with-* flag requires it and
fails when the system or release cannot provide it, where a default is skipped.
Folder association also installs the desktop entry and "Open file location".
EOF
}

parse_args() {
  while (($# > 0)); do
    case $1 in
      --non-interactive) NON_INTERACTIVE=yes ;;
      --verbose) VERBOSE=yes ;;
      --with-smb) NON_INTERACTIVE=yes; WITH_SMB=yes ;;
      --with-raw) NON_INTERACTIVE=yes; WITH_RAW=yes ;;
      --with-desktop-entry) NON_INTERACTIVE=yes; WITH_DESKTOP_ENTRY=yes ;;
      --with-folder-association)
        NON_INTERACTIVE=yes
        WITH_DESKTOP_ENTRY=yes
        WITH_FOLDER_ASSOCIATION=yes
        WITH_FILE_MANAGER=yes
        ;;
      --with-file-manager) NON_INTERACTIVE=yes; WITH_FILE_MANAGER=yes ;;
      --with-file-chooser) NON_INTERACTIVE=yes; WITH_FILE_CHOOSER=yes; REQUESTED_FILE_CHOOSER=yes ;;
      --without-file-chooser) NON_INTERACTIVE=yes; WITH_FILE_CHOOSER=no ;;
      --with-omarchy-keybinds) NON_INTERACTIVE=yes; WITH_OMARCHY_KEYBINDS=yes ;;
      --without-omarchy-keybinds) NON_INTERACTIVE=yes; WITH_OMARCHY_KEYBINDS=no ;;
      --with-udiskie-unlock) NON_INTERACTIVE=yes; WITH_UDISKIE_UNLOCK=yes; REQUESTED_UDISKIE_UNLOCK=yes ;;
      --without-udiskie-unlock) NON_INTERACTIVE=yes; WITH_UDISKIE_UNLOCK=no ;;
      --without-smb) NON_INTERACTIVE=yes; WITH_SMB=no ;;
      --without-raw) NON_INTERACTIVE=yes; WITH_RAW=no ;;
      --without-desktop-entry) NON_INTERACTIVE=yes; WITH_DESKTOP_ENTRY=no; WITH_FOLDER_ASSOCIATION=no ;;
      --without-folder-association) NON_INTERACTIVE=yes; WITH_FOLDER_ASSOCIATION=no ;;
      --without-file-manager) NON_INTERACTIVE=yes; WITH_FILE_MANAGER=no; WITH_FOLDER_ASSOCIATION=no ;;
      -h | --help) usage; exit 0 ;;
      *) die "Unknown option: $1 (run with --help for usage)." ;;
    esac
    shift
  done
}

version_at_least() {
  local actual=$1 required=$2 first
  first=$(printf '%s\n%s\n' "$required" "$actual" | sort -V | head -n 1)
  [[ $first == "$required" ]]
}

detect_target() {
  case $(uname -m) in
    x86_64 | amd64) printf '%s\n' x86_64-unknown-linux-gnu ;;
    aarch64 | arm64) printf '%s\n' aarch64-unknown-linux-gnu ;;
    *) die "Strata has no prebuilt release for $(uname -m)." ;;
  esac
}

omarchy_major_from() {
  if [[ $1 =~ (^|[^0-9.])([34])[.][0-9]+ ]]; then
    printf '%s\n' "${BASH_REMATCH[2]}"
    return 0
  fi
  return 1
}

detect_omarchy_major() {
  local output="" version_file

  if command -v omarchy >/dev/null 2>&1; then
    output=$(omarchy version 2>/dev/null || true)
    if omarchy_major_from "$output"; then
      return 0
    fi
  fi

  for version_file in /usr/share/omarchy/version "$HOME/.local/share/omarchy/version"; do
    if [[ -r $version_file ]] && omarchy_major_from "$(<"$version_file")"; then
      return 0
    fi
  done

  return 0
}

latest_stable_version() {
  local effective tag
  effective=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
    "https://github.com/$REPOSITORY/releases/latest") \
    || die "Could not find the latest stable Strata release."
  tag=${effective##*/}
  [[ $tag =~ ^v([0-9]+[.][0-9]+[.][0-9]+)$ ]] \
    || die "GitHub returned an unexpected stable release tag: $tag"
  printf '%s\n' "${BASH_REMATCH[1]}"
}

run_pacman() {
  if [[ $NON_INTERACTIVE == yes ]]; then
    TASK_OUTPUT=live run_task "$(text dependencies)" sudo -n pacman -S --needed --noconfirm -- "$@" \
      || die "Non-interactive package installation failed; passwordless sudo or cached credentials may be required."
  else
    TASK_OUTPUT=live run_task "$(text dependencies)" sudo pacman -S --needed -- "$@" </dev/tty
  fi
}

install_arch_dependencies() {
  local missing=() requested=("${REQUIRED_PACKAGES[@]}") package
  [[ $WITH_SMB != yes ]] || requested+=(gvfs-smb)
  [[ $WITH_RAW != yes ]] || requested+=("${RAW_PREVIEW_PACKAGES[@]}")
  [[ $WITH_FILE_CHOOSER != yes ]] || requested+=(xdg-desktop-portal)
  for package in "${requested[@]}"; do
    [[ $package == github-cli ]] && command -v gh >/dev/null 2>&1 && continue
    pacman -Q "$package" >/dev/null 2>&1 || missing+=("$package")
  done

  if ((${#missing[@]} == 0)); then
    info "$(text dependencies_ready)"
    return
  fi

  section "$(text dependencies)"
  printf '  %s\n\n' "$(text packages "packages=${missing[*]}")"
  run_pacman "${missing[@]}"
}

install_desktop_entry() {
  local extracted=$1 make_default=$2 desktop_dir icon_dir escaped_bin staged
  desktop_dir=${XDG_DATA_HOME:-$HOME/.local/share}/applications
  icon_dir=${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor
  staged=$TEMP_DIR/$APP_ID.desktop
  escaped_bin=${BIN_PATH//&/\\&}

  install -Dm644 "$extracted/$APP_ID.svg" \
    "$icon_dir/scalable/apps/$APP_ID.svg"
  sed "s|^Exec=strata |Exec=$escaped_bin |" "$extracted/$APP_ID.desktop" >"$staged"
  install -Dm644 "$staged" "$desktop_dir/$APP_ID.desktop"

  command -v update-desktop-database >/dev/null 2>&1 \
    && update-desktop-database "$desktop_dir" 2>/dev/null || true
  command -v gtk-update-icon-cache >/dev/null 2>&1 \
    && gtk-update-icon-cache -qtf "$icon_dir" 2>/dev/null || true
  info "Added Strata to the desktop application menu."

  if [[ $make_default == yes ]]; then
    command -v xdg-mime >/dev/null 2>&1 \
      || die "xdg-mime is required to change the folder association."
    timeout 5 xdg-mime default "$APP_ID.desktop" inode/directory
    local current
    current=$(timeout 5 xdg-mime query default inode/directory)
    [[ $current == "$APP_ID.desktop" ]] \
      || die "The folder association did not change (current value: $current)."
    info "Strata is now the default application for folders."
  fi
}

install_file_manager_service() {
  local extracted=$1 service_dir target conflict escaped_bin staged
  service_dir=${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services
  target=$service_dir/$APP_ID.FileManager1.service

  [[ -r $extracted/$APP_ID.FileManager1.service ]] \
    || die "The verified archive is missing FileManager1 integration."
  install -d "$service_dir"
  conflict=$(grep -l '^Name=org\.freedesktop\.FileManager1$' \
    "$service_dir"/*.service 2>/dev/null \
    | grep -v "/$APP_ID.FileManager1.service$" | head -n 1 || true)
  [[ -z $conflict ]] \
    || die "Another per-user FileManager1 provider is already installed: $conflict"

  staged=$TEMP_DIR/$APP_ID.FileManager1.service
  escaped_bin=${BIN_PATH//&/\\&}
  sed "s|^Exec=/usr/bin/strata |Exec=$escaped_bin |" \
    "$extracted/$APP_ID.FileManager1.service" >"$staged"
  install -Dm644 "$staged" "$target"
  if timeout 5 gdbus call --session --timeout 5 \
    --dest org.freedesktop.DBus \
    --object-path /org/freedesktop/DBus \
    --method org.freedesktop.DBus.ReloadConfig >/dev/null 2>&1; then
    info "$(text reveal_ready)"
  else
    warn "$(text reveal_relogin)"
  fi
}

configure_omarchy_bindings() {
  local major=$1 bindings backup errors
  if [[ $major == 4 ]]; then
    bindings=$HOME/.config/hypr/bindings.lua
  else
    bindings=$HOME/.config/hypr/bindings.conf
  fi

  install -d "$(dirname "$bindings")"
  if grep -q 'strata-installer: file-manager start' "$bindings" 2>/dev/null; then
    info "Omarchy file-manager keybinds already point to Strata."
    return
  fi

  backup=$bindings.bak.$(date +%Y%m%d%H%M%S)
  if [[ -e $bindings ]]; then
    cp -p "$bindings" "$backup"
  else
    : >"$bindings"
    backup=""
  fi

  if [[ $major == 4 ]]; then
    cat >>"$bindings" <<EOF

-- strata-installer: file-manager start
hl.unbind("SUPER + SHIFT + F")
hl.unbind("SUPER + ALT + SHIFT + F")
o.bind("SUPER + SHIFT + F", "File manager", { launch = "$BIN_PATH" })
o.bind("SUPER + ALT + SHIFT + F", "File manager (cwd)",
  "uwsm-app -- $BIN_PATH \"\$(omarchy-cmd-terminal-cwd)\"")
-- strata-installer: file-manager end
EOF
  else
    cat >>"$bindings" <<EOF

# strata-installer: file-manager start
unbind = SUPER SHIFT, F
unbind = SUPER ALT SHIFT, F
bindd = SUPER SHIFT, F, File manager, exec, uwsm-app -- $BIN_PATH
bindd = SUPER ALT SHIFT, F, File manager (cwd), exec, uwsm-app -- $BIN_PATH "\$(omarchy-cmd-terminal-cwd)"
# strata-installer: file-manager end
EOF
  fi

  if command -v hyprctl >/dev/null 2>&1 && [[ -n ${HYPRLAND_INSTANCE_SIGNATURE:-} ]]; then
    hyprctl reload >/dev/null
    errors=$(hyprctl configerrors 2>&1 || true)
    if [[ -n ${errors//[[:space:]]/} ]]; then
      if [[ -n $backup ]]; then
        cp -p "$backup" "$bindings"
      else
        rm -f "$bindings"
      fi
      hyprctl reload >/dev/null 2>&1 || true
      die "Hyprland rejected the keybind change; restored the previous config:"$'\n'"$errors"
    fi
  else
    warn "Hyprland is not running, so the keybind file could not be reloaded now."
  fi

  info "Omarchy $major file-manager shortcuts now open Strata."
  [[ -n $backup ]] && printf 'Backup: %s\n' "$backup"
  return 0
}

udiskie_on_path() {
  command -v udiskie >/dev/null 2>&1
}

configure_file_chooser() {
  local extracted=$1
  if [[ $WITH_FILE_CHOOSER == yes ]]; then
    TASK_OUTPUT=notices run_task "$(text chooser)" "$BIN_PATH" --install-portal \
      || die "File chooser setup failed. Strata is installed; retry in Settings → General → Desktop integration → System file manager."
  elif [[ -r $extracted/portal/strata.portal && ( $NON_INTERACTIVE == no || $WITH_FILE_CHOOSER == no ) ]]; then
    "$BIN_PATH" --dismiss-portal-prompt >>"${LOG_FILE:-/dev/null}" 2>&1 \
      || warn "Could not save your choice. Strata may offer file chooser integration on first launch."
  fi
}

main() {
  local target glibc distro_id distro_name distro_like omarchy_major version archive extracted url
  local local_bin_on_path=no arch_based=no command

  parse_args "$@"
  [[ $(uname -s) == Linux ]] || die "The prebuilt Strata release supports Linux only."
  [[ $EUID -ne 0 ]] || die "Run this installer as your normal desktop user, not as root."
  if [[ $NON_INTERACTIVE == no ]]; then
    [[ -e /dev/tty && -r /dev/tty && -w /dev/tty ]] \
      || die "This interactive installer needs a terminal."
    PROMPT_DEVICE=/dev/tty
  fi
  setup_terminal
  trap installer_cleanup EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  show_banner

  target=$(detect_target)
  command -v getconf >/dev/null 2>&1 || die "Could not detect the system C library."
  glibc=$(getconf GNU_LIBC_VERSION 2>/dev/null | awk '{print $2}')
  [[ $glibc =~ ^[0-9]+[.][0-9]+ ]] || die "Strata requires a glibc-based Linux system."
  version_at_least "$glibc" "$MIN_GLIBC" \
    || die "Strata requires glibc $MIN_GLIBC or newer (found $glibc)."

  distro_id="unknown"
  distro_like=""
  if [[ -r /etc/os-release ]]; then
    # shellcheck disable=SC1091
    source /etc/os-release
    distro_id=${ID:-unknown}
    distro_like=${ID_LIKE:-}
  fi
  distro_name=${PRETTY_NAME:-$distro_id}
  omarchy_major=$(detect_omarchy_major)
  [[ $WITH_OMARCHY_KEYBINDS != yes || -n $omarchy_major ]] \
    || die "--with-omarchy-keybinds requires Omarchy 3 or 4."
  if [[ $distro_id == arch || $distro_like == *arch* || -n $omarchy_major ]]; then
    command -v pacman >/dev/null 2>&1 || die "This Arch-based system does not provide pacman."
    arch_based=yes
  elif [[ $NON_INTERACTIVE == yes ]]; then
    die "Non-interactive dependency installation currently supports Arch-based systems only."
  fi
  if [[ $WITH_UDISKIE_UNLOCK == yes ]]; then
    [[ $arch_based == yes ]] && udiskie_on_path \
      || die "$(text udiskie_requires)"
  fi
  resolve_default_integrations "$arch_based" "$omarchy_major"
  for command in curl tar sha256sum openssl wc install sed timeout; do
    command -v "$command" >/dev/null 2>&1 || die "Required command not found: $command"
  done

  BIN_PATH=$HOME/.local/bin/strata
  [[ :$PATH: == *":$HOME/.local/bin:"* ]] && local_bin_on_path=yes
  printf '\n  %s%s · %s%s\n' "$COLOR_DIM" "$distro_name" "${target%%-unknown-*}" "$COLOR_RESET"
  if [[ -e $BIN_PATH ]]; then
    [[ $NON_INTERACTIVE == no ]] \
      || die "$BIN_PATH already exists; remove it or run the interactive installer to replace it."
    printf '\n  %s\n' "$(text reinstall "path=$BIN_PATH")"
  fi
  choose_installation "$arch_based" "$omarchy_major"
  show_installation_plan
  if [[ $arch_based != yes ]]; then
    printf '\nStrata needs GTK 4.12+, GtkSourceView 5, Poppler GLib, Fontconfig, Bubblewrap,\n'
    printf 'FFmpeg, ffmpegthumbnailer, GStreamer plugins, and the GVfs UDisks2 volume monitor.\n'
    printf 'For the volume monitor, install gvfs-daemons on Debian/Ubuntu or gvfs on Fedora.\n'
    prompt "Have you installed the equivalent packages for this distribution?" \
      || die "Install the runtime dependencies, then run this installer again."
  fi

  TEMP_DIR=$(mktemp -d)
  LOG_FILE=$(mktemp "${TMPDIR:-/tmp}/strata-install.XXXXXX.log")
  section "$(text installing)"
  version=$(latest_stable_version)
  archive="strata-$version-$target.tar.gz"
  url="https://github.com/$REPOSITORY/releases/download/v$version"
  download_release_file "$(text download_manifest)" "$url/$MANIFEST_NAME" \
    "$TEMP_DIR/$MANIFEST_NAME" "$MAX_MANIFEST_BYTES"
  download_release_file "$(text download_signatures)" "$url/$SIGNATURES_NAME" \
    "$TEMP_DIR/$SIGNATURES_NAME" "$MAX_SIGNATURES_BYTES"
  run_task "$(text signature)" verify_release_signature "$TEMP_DIR/$MANIFEST_NAME" "$TEMP_DIR/$SIGNATURES_NAME"
  read_signed_artifact "$TEMP_DIR/$MANIFEST_NAME" "$version" "$target" "$archive"
  local -a download_flags=(--silent)
  if [[ $RICH_TERMINAL == yes && $VERBOSE == no ]]; then download_flags=(--progress-bar); fi
  TASK_PROGRESS=yes run_task "$(text download "version=$version")" curl --fail --location --show-error \
    "${download_flags[@]}" --max-filesize "$SIGNED_ARCHIVE_SIZE" --output "$TEMP_DIR/$archive" "$url/$archive"
  run_task "$(text digest)" verify_archive_digest "$TEMP_DIR/$archive"
  run_task "$(text unpack)" tar -xzf "$TEMP_DIR/$archive" -C "$TEMP_DIR"

  extracted=$TEMP_DIR/${archive%.tar.gz}
  [[ -x $extracted/strata ]] || die "The verified archive does not contain the Strata binary."
  [[ -r $extracted/$APP_ID.desktop && -r $extracted/$APP_ID.svg ]] \
    || die "The verified archive is missing desktop integration files."
  if [[ $WITH_FILE_CHOOSER == yes && ! -r $extracted/portal/strata.portal ]]; then
    [[ $REQUESTED_FILE_CHOOSER != yes ]] \
      || die "This release does not include file chooser integration. Install a newer release."
    warn "This release does not include file chooser integration; skipping it."
    WITH_FILE_CHOOSER=no
  fi
  if [[ $WITH_UDISKIE_UNLOCK == yes && ! -r $extracted/udiskie/unlock ]]; then
    [[ $REQUESTED_UDISKIE_UNLOCK != yes ]] \
      || die "This release does not include encrypted-volume integration. Install a newer release."
    warn "This release does not include encrypted-volume integration; skipping it."
    WITH_UDISKIE_UNLOCK=no
  fi
  if [[ $arch_based == yes ]]; then
    install_arch_dependencies
  fi
  run_task "$(text application)" install -Dm755 "$extracted/strata" "$BIN_PATH"
  CORE_INSTALLED=yes
  export PATH="$HOME/.local/bin:$PATH"

  if [[ $WITH_DESKTOP_ENTRY == yes ]]; then
    install_desktop_entry "$extracted" "$WITH_FOLDER_ASSOCIATION"
  fi
  if [[ $WITH_FILE_MANAGER == yes ]]; then
    install_file_manager_service "$extracted"
  fi
  configure_file_chooser "$extracted"
  if [[ $WITH_OMARCHY_KEYBINDS == yes ]]; then
    configure_omarchy_bindings "$omarchy_major"
  fi
  if [[ $WITH_UDISKIE_UNLOCK == yes ]]; then
    TASK_OUTPUT=notices run_task "$(text udiskie)" "$BIN_PATH" --install-udiskie-unlock \
      || die "Encrypted-volume unlock setup failed. Strata is installed; retry with: $BIN_PATH --install-udiskie-unlock"
  fi

  if [[ $local_bin_on_path == no ]]; then
    warn "$HOME/.local/bin is not on PATH in this shell."
  fi
  if ((WARNING_COUNT > 0)); then
    COLOR_ACCENT=$COLOR_WARNING section "$(text installed_warnings)"
  elif [[ $SETUP_NOTICES == yes ]]; then
    COLOR_ACCENT=$COLOR_WARNING section "$(text installed_notices)"
  else
    COLOR_ACCENT=$COLOR_SUCCESS section "$(text ready)"
  fi
  info "$(text installed "version=$version")"
  printf '\n  %s\n' "$(text launch "command=$BIN_PATH")"
  if ((WARNING_COUNT > 0)) || [[ $SETUP_NOTICES == yes ]]; then
    printf '\n  %s%s%s\n' "$COLOR_DIM" "$(text details "path=$LOG_FILE")" "$COLOR_RESET"
  fi
  printf '\n'
}

if [[ ${STRATA_INSTALLER_TESTING:-0} != 1 ]]; then
  main "$@"
fi
