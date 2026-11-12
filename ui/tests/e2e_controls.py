"""Browser regression tests against the actual Rust v1 host.

Only the delayed-HTTP and WebSocket fault cases alter transport, through
Playwright routes in the test process. No production endpoints are invented.
Artifacts are external to the checkout. Run cases sequentially: engine state is
shared. This runner returns nonzero when any assertion or browser error fails.
"""
import argparse
import asyncio
import hashlib
import json
import pathlib
import time
import traceback
import urllib.request

from playwright.async_api import async_playwright, expect


class Suite:
    def __init__(self, base, output):
        self.base, self.output = base, pathlib.Path(output)
        self.results = []

    def api(self, command=None):
        request = self.base + "/api/v1/state"
        if command is not None:
            request = urllib.request.Request(
                self.base + "/api/v1/command", data=json.dumps(command).encode(),
                headers={"Content-Type": "application/json"},
            )
        with urllib.request.urlopen(request, timeout=5) as response:
            if command is None:
                return json.load(response)
            assert response.status == 202

    async def wait_engine(self, predicate, message, timeout=5):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            state = self.api()
            if predicate(state):
                return state
            await asyncio.sleep(.05)
        raise AssertionError(message)

    async def open(self, page):
        self.api({"type": "panic"})
        self.api({"type": "preset", "index": 0})
        await asyncio.sleep(.15)
        await page.goto(self.base)
        await page.get_by_text("RUST ENGINE CONNECTED", exact=False).wait_for()
        await expect(page.get_by_role("slider", name="Filter Cutoff", exact=True)).to_be_enabled()

    @staticmethod
    async def value(slider):
        return float(await slider.get_attribute("aria-valuenow"))

    async def edit(self, page, name, value):
        await page.get_by_role("button", name="Edit " + name, exact=True).click()
        field = page.get_by_role("textbox", name=name + " value", exact=True)
        await field.fill(str(value))
        await field.press("Enter")

    async def numeric(self, page):
        await self.open(page)
        slider = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        await self.edit(page, "Filter Cutoff", 1234.5)
        await expect(slider).to_have_attribute("aria-valuenow", "1234.5")
        await self.wait_engine(lambda s: abs(s["values"][23]-1234.5)<.01, "Numeric edit did not reach Rust")
        await page.get_by_role("button", name="Edit Filter Cutoff", exact=True).click()
        field = page.get_by_role("textbox", name="Filter Cutoff value", exact=True)
        await field.fill("20000")
        await field.press("Enter")
        await expect(field).to_be_visible()
        await expect(page.get_by_role("alert")).to_contain_text("20")
        await field.fill("900")
        await field.press("Escape")
        assert abs(await self.value(slider)-1234.5)<.01, "Escape committed a numeric edit"
        await page.get_by_role("button", name="UNDO", exact=True).click()
        await self.wait_engine(lambda s: abs(s["values"][23]-680)<.01, "Undo numeric edit failed")
        await page.get_by_role("button", name="REDO", exact=True).click()
        await self.wait_engine(lambda s: abs(s["values"][23]-1234.5)<.01, "Redo numeric edit failed")

    async def drag_undo(self, page):
        await self.open(page)
        slider = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        before = await self.value(slider)
        await slider.scroll_into_view_if_needed()
        rect = await slider.bounding_box()
        x, y = rect["x"]+rect["width"]/2, rect["y"]+rect["height"]/2
        await page.mouse.move(x, y)
        await page.mouse.down()
        await page.mouse.move(x, y-35, steps=12)
        await page.mouse.up()
        await page.wait_for_timeout(300)
        changed = await self.value(slider)
        assert abs(changed-before)>1
        await page.get_by_role("button", name="UNDO", exact=True).click()
        await self.wait_engine(lambda s: abs(s["values"][23]-before)<.01, "One drag was not one undo")
        await page.get_by_role("button", name="REDO", exact=True).click()
        await self.wait_engine(lambda s: abs(s["values"][23]-changed)<.01, "Drag redo failed")

    async def touch_keyboard(self, page):
        await self.open(page)
        slider = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        await slider.scroll_into_view_if_needed()
        rect = await slider.bounding_box()
        before = await self.value(slider)
        x, y = rect["x"]+rect["width"]/2, rect["y"]+rect["height"]/2
        cdp = await page.context.new_cdp_session(page)
        await cdp.send("Input.dispatchTouchEvent", {"type":"touchStart", "touchPoints":[{"x":x,"y":y,"id":1}]})
        for dy in (5,10,15,20):
            await cdp.send("Input.dispatchTouchEvent", {"type":"touchMove", "touchPoints":[{"x":x,"y":y-dy,"id":1}]})
        await cdp.send("Input.dispatchTouchEvent", {"type":"touchEnd", "touchPoints":[]})
        await page.wait_for_timeout(250)
        changed = await self.value(slider)
        assert abs(changed-before)>1, "Touch pointer gesture did not change knob"
        await page.get_by_role("button", name="UNDO", exact=True).click()
        await self.wait_engine(lambda s: abs(s["values"][23]-before)<.01, "Touch gesture undo failed")
        await slider.focus()
        await slider.press("ArrowUp")
        await self.wait_engine(lambda s: s["values"][23]>before, "Keyboard knob nudge did not reach Rust")
        key = page.get_by_role("button", name="MIDI note 60", exact=True)
        await key.focus()
        await page.keyboard.down("Space")
        playing = await self.wait_engine(lambda s: s["activeVoiceMask"]!=0 and max(abs(v) for v in s["scope"])>.001, "Keyboard note produced no Rust output")
        assert playing["activeVoiceMask"].bit_count() == 1
        await page.keyboard.up("Space")
        await self.wait_engine(lambda s: s["activeVoiceMask"]==0, "Keyboard NoteOff did not release voice")
        await key.scroll_into_view_if_needed()
        rect = await key.bounding_box()
        x,y=rect["x"]+rect["width"]/2,rect["y"]+rect["height"]/2
        await cdp.send("Input.dispatchTouchEvent", {"type":"touchStart", "touchPoints":[{"x":x,"y":y,"id":2}]})
        await self.wait_engine(lambda s:s["activeVoiceMask"]!=0, "Touch keyboard did not issue NoteOn")
        await cdp.send("Input.dispatchTouchEvent", {"type":"touchCancel", "touchPoints":[]})
        await self.wait_engine(lambda s:s["activeVoiceMask"]==0, "Touch cancel left a stuck voice")
        await cdp.detach()

    async def delayed_controls(self, page):
        await self.open(page)
        self.api({"type":"param","id":46,"value":0})
        await page.wait_for_timeout(200)
        async def delay(route):
            if route.request.post_data_json.get("type") == "param":
                await asyncio.sleep(.45)
            await route.continue_()
        await page.route("**/api/v1/command*", delay)
        slider = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        await slider.focus()
        await slider.press("ArrowUp")
        optimistic = await self.value(slider)
        await page.wait_for_timeout(180)
        assert abs(await self.value(slider)-optimistic)<.01, "Stale WebSocket telemetry overwrote local intent"
        await page.wait_for_timeout(700)
        await page.get_by_role("button", name="CHORUS I", exact=True).click()
        await page.wait_for_timeout(100)
        await page.get_by_role("button", name="CHORUS II", exact=True).click()
        mode = page.get_by_role("slider", name="Chorus Mode", exact=True)
        assert await self.value(mode)==3, "Rapid Chorus buttons did not share one atomic mode"
        await self.wait_engine(lambda s:s["values"][46]==3, "Chorus intents were lost before reaching Rust")
        await page.unroute("**/api/v1/command*", delay)

    async def dependencies(self, page):
        await self.open(page)
        pw = page.get_by_role("slider", name="OscA Pw", exact=True)
        await expect(pw).to_be_disabled()
        await page.get_by_role("switch", name="OscA Pulse", exact=True).click()
        await expect(pw).to_be_enabled()
        sync = page.get_by_role("switch", name="Reverb Pre Delay Sync", exact=True)
        bpm = page.get_by_role("slider", name="Reverb Tempo Bpm", exact=True)
        predelay = page.get_by_role("slider", name="Reverb Pre Delay", exact=True)
        await expect(bpm).to_be_disabled()
        await expect(predelay).to_be_enabled()
        await sync.click()
        await expect(bpm).to_be_enabled()
        await expect(predelay).to_be_disabled()
        for name in ("Reverb Shimmer", "Reverb Vowel Mix", "Reverb Mod Depth"):
            await self.edit(page, name, 0)
        for name in ("Shimmer Fifth", "Shimmer Semitones", "Reverb Vowel Morph", "Reverb Mod Speed"):
            await expect(page.get_by_role("slider", name=name, exact=True)).to_be_disabled()

    async def envelopes(self, page):
        await self.open(page)
        graph = page.get_by_test_id("envelope-filter").locator("path").first
        await self.edit(page,"Filt Attack", .001)
        short = await graph.get_attribute("d")
        await self.edit(page,"Filt Attack", 1)
        long = await graph.get_attribute("d")
        assert short != long, "Attack did not change programmed geometry"
        await self.edit(page,"Filt Release", .01)
        short = await graph.get_attribute("d")
        await self.edit(page,"Filt Release", 5)
        long = await graph.get_attribute("d")
        assert short != long, "Release did not change programmed geometry"
        await expect(page.get_by_text("Envelope stages unavailable on protocol v1", exact=False).first).to_be_visible()
        assert await page.get_by_test_id("envelope-filter").locator("circle:not([role=slider])").count()==0, "V1 levels falsely drawn as temporal stages"

    async def unavailable_v1(self, page):
        await self.open(page)
        slider = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        await slider.click(button="right")
        dialog = page.get_by_role("dialog", name="MIDI Learn", exact=True)
        await expect(dialog).to_be_visible()
        for name in ("Cancel MIDI Learn", "Remove MIDI mapping", "Replace MIDI mapping"):
            await expect(dialog.get_by_role("button", name=name, exact=True)).to_be_disabled()
        async with page.expect_response(lambda response: "/api/v1/command" in response.url and response.request.post_data_json.get("type")=="learn") as response_info:
            await dialog.get_by_role("button", name="Start MIDI Learn", exact=True).click()
        response=await response_info.value
        assert response.status==202
        await asyncio.sleep(.1)  # v1 has no capture-ready acknowledgment; allow the existing 10-ms relay/RT queue to apply it.
        self.api({"type":"midi","status":176,"data1":16,"data2":64})
        expected = 20*900**(64/127)
        await self.wait_engine(lambda s:abs(s["values"][23]-expected)<1, "V1 MIDI Learn assignment failed")
        # Assignment feedback is not fabricated from changed parameter values.
        await expect(page.get_by_text("unavailable", exact=False).first).to_be_visible()

    async def reconnect(self, page):
        sockets = []
        def wire(route):
            sockets.append(route)
            route.connect_to_server()
        await page.route_web_socket("**/api/v1/ws*", wire)
        await self.open(page)
        assert sockets
        sockets[-1].send("{malformed")
        sockets[-1].send(json.dumps({"v":1,"ready":True,"values":[]}))
        await page.wait_for_timeout(150)
        await expect(page.get_by_role("slider", name="Filter Cutoff", exact=True)).to_be_enabled()
        await sockets[-1].close(code=1001,reason="test disconnect")
        await expect(page.get_by_role("slider", name="Filter Cutoff", exact=True)).to_be_disabled()
        await expect(page.get_by_role("slider", name="Filter Cutoff", exact=True)).to_be_enabled(timeout=8000)
        assert len(sockets)>=2, "No reconnection WebSocket was created"


    async def run(self, chromium, selected=None):
        self.output.mkdir(parents=True, exist_ok=True)
        cases = [self.numeric,self.drag_undo,self.touch_keyboard,self.delayed_controls,self.dependencies,self.envelopes,self.unavailable_v1,self.reconnect]
        if selected:
            unknown=set(selected)-{case.__name__ for case in cases}
            if unknown:raise ValueError("Unknown cases: "+", ".join(sorted(unknown)))
            cases=[case for case in cases if case.__name__ in selected]
        async with async_playwright() as playwright:
            browser = await playwright.chromium.launch(executable_path=chromium,headless=True,args=["--no-sandbox"])
            for case in cases:
                context = await browser.new_context(viewport={"width":1440,"height":1000},has_touch=True)
                page = await context.new_page()
                errors=[]
                page.on("pageerror",lambda error:errors.append(str(error)))
                started=time.monotonic()
                try:
                    await case(page)
                    assert not errors,errors
                    result={"case":case.__name__,"status":"PASS"}
                except Exception as error:
                    result={"case":case.__name__,"status":"FAIL","error":str(error),"traceback":traceback.format_exc(),"browser_errors":errors}
                result["seconds"]=round(time.monotonic()-started,3)
                self.results.append(result)
                await page.screenshot(path=str(self.output/(case.__name__+".png")),full_page=True)
                await context.close()
                self.api({"type":"panic"})
                print(json.dumps(result),flush=True)
            await browser.close()
        report={"backend":"actual Rust protocol v1; explicit test-only fault injection in delayed_controls/reconnect","base":self.base,"results":self.results,"status":"FAIL" if any(x["status"]=="FAIL" for x in self.results) else "PASS"}
        (self.output/"e2e-controls.json").write_text(json.dumps(report,indent=2)+"\n")
        files=sorted(p for p in self.output.iterdir() if p.is_file() and p.name!="SHA256SUMS")
        (self.output/"SHA256SUMS").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in files))
        return 1 if report["status"]=="FAIL" else 0


if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--base",default="http://127.0.0.1:8787")
    parser.add_argument("--output",default="/workspace/.prophet-cloud/task-a/e2e-controls")
    parser.add_argument("--chromium",default="/usr/bin/chromium")
    parser.add_argument("--case",action="append",help="Run only a named case; repeat to select several")
    args=parser.parse_args()
    raise SystemExit(asyncio.run(Suite(args.base,args.output).run(args.chromium,args.case)))
