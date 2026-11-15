// Armed before spotify-launcher starts; no persistent KWin rules or shortcuts.
let stopped = false;
let foundAt = 0;
const deadline = Date.now() + 30000;
const timer = new QTimer();
timer.interval = 100;

function isSpotify(window) {
    return !window.specialWindow && !window.transient &&
        [window.resourceClass, window.resourceName, window.desktopFileName].some(function(value) {
            return /^(spotify|com\.spotify\.client)$/i.test(String(value || ""));
        });
}

function stop() {
    if (stopped) { return; }
    stopped = true;
    timer.stop();
    workspace.windowAdded.disconnect(onWindowAdded);
    workspace.windowRemoved.disconnect(onWindowRemoved);
    workspace.windowActivated.disconnect(onWindowActivated);
    callDBus("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "unloadScript", settings.plugin);
}

function minimize(window) {
    if (stopped || !isSpotify(window)) { return; }
    window.minimized = true;
    const home = workspace.windowList().find(function(window) { return window.pid === settings.shellPid; });
    if (home) {
        home.minimized = false;
        workspace.activeWindow = home;
    }
    if (!foundAt) { foundAt = Date.now(); }
}

function onWindowAdded(window) { minimize(window); }
function onWindowRemoved(window) { if (window.pid === settings.shellPid) { stop(); } }
function onWindowActivated(window) { if (window) { minimize(window); } }

function tick() {
    if (stopped) {return;}
    if (Date.now() >= deadline || (foundAt && Date.now() - foundAt > 2000)) {stop(); return;}
    // Some XWayland windows publish their app ID after windowAdded.
    workspace.windowList().forEach(function(window) {
        if (isSpotify(window) && !window.minimized) {minimize(window);}
    });
}

workspace.windowAdded.connect(onWindowAdded);
workspace.windowRemoved.connect(onWindowRemoved);
workspace.windowActivated.connect(onWindowActivated);
timer.timeout.connect(tick);
timer.start();
workspace.windowList().forEach(minimize);
