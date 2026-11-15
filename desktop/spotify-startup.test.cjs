const {test} = require("node:test");
const assert = require("node:assert/strict");
const vm = require("node:vm");
const source = require("node:fs").readFileSync(`${__dirname}/spotify-startup.js`, "utf8");

function signal() {
    let listeners = [];
    return {
        connect(fn) {listeners.push(fn);},
        disconnect(fn) {listeners = listeners.filter(value => value !== fn);},
        emit(value) {listeners.slice().forEach(fn => fn(value));}
    };
}

function setup(existing = []) {
    let now = 1000;
    let timer;
    let unloaded = false;
    const home = {pid:42,minimized:false};
    const workspace = {windowAdded:signal(),windowRemoved:signal(),windowActivated:signal(),windows:[home,...existing]};
    workspace.windowList = () => workspace.windows;
    let active = home;
    Object.defineProperty(workspace,"activeWindow", {
        get() {return active;},
        set(value) {active=value; workspace.windowActivated.emit(value);}
    });
    vm.runInNewContext(source, {
        settings:{shellPid:42,plugin:"test"},workspace,Date:{now:()=>now},
        QTimer:function() {timer=this;this.timeout=signal();this.start=()=>{};this.stop=()=>{};},
        callDBus(service,path,iface,method,plugin) {
            assert.equal(method,"unloadScript");assert.equal(plugin,"test");unloaded=true;
        }
    });
    return {
        home,workspace,
        add(window) {workspace.windows.push(window);workspace.windowAdded.emit(window);},
        advance(ms) {now+=ms;timer.timeout.emit();},
        removeHome() {workspace.windowRemoved.emit(home);},
        unloaded:()=>unloaded
    };
}
const spotify = () => ({pid:100,resourceClass:"Spotify",minimized:false});

test("new Spotify window is minimized and cannot steal startup focus", () => {
    const env=setup(); const player=spotify();env.add(player);
    assert.equal(player.minimized,true); assert.equal(env.workspace.activeWindow,env.home);
    player.minimized=false; env.workspace.activeWindow=player;
    assert.equal(player.minimized,true); assert.equal(env.workspace.activeWindow,env.home);
});
test("an existing Spotify window is minimized when the TV tile launches", () => {
    const player=spotify();setup([player]);assert.equal(player.minimized,true);
});
test("normal desktop behavior returns after startup", () => {
    const env=setup();const player=spotify();env.add(player);env.advance(2100);
    assert.equal(env.unloaded(),true);
    player.minimized=false;env.workspace.activeWindow=player;
    assert.equal(player.minimized,false);assert.equal(env.workspace.activeWindow,player);
});
test("unrelated applications are untouched and unmatched requests expire", () => {
    const env=setup();const browser={pid:99,resourceClass:"firefox",minimized:false};env.add(browser);
    env.workspace.activeWindow=browser;
    assert.equal(browser.minimized,false);assert.equal(env.workspace.activeWindow,browser);
    env.advance(31000);assert.equal(env.unloaded(),true);
    const late=spotify();env.add(late);assert.equal(late.minimized,false);
});
test("late app IDs and Flatpak app IDs are recognized; closing the shell disarms", () => {
    const env=setup();const player={pid:100,resourceClass:"",desktopFileName:"",minimized:false};env.add(player);
    player.desktopFileName="com.spotify.Client";env.advance(100);
    assert.equal(player.minimized,true);
    env.removeHome();assert.equal(env.unloaded(),true);
});
