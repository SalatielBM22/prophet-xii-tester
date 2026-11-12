"""Reproduce the original UI defects without modifying the engine or tracked evidence.

Run against a host serving the b47f2d6 frontend. This is an observation probe:
defects are reported as FAIL against the desired behavior, not passing tests.
HTTP delays are injected by Playwright only in this test process.
"""
import argparse
import asyncio
import hashlib
import json
import pathlib
import re
import subprocess
import urllib.request

from playwright.async_api import async_playwright

BASELINE = "b47f2d60c7005cd6c81694c0e17c76f7a4f107dc"
REPO = pathlib.Path(__file__).resolve().parents[2]


def api(base, command=None):
    if command is None:
        with urllib.request.urlopen(base + "/api/v1/state", timeout=5) as response:
            return json.load(response)
    request = urllib.request.Request(
        base + "/api/v1/command", data=json.dumps(command).encode(),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=5) as response:
        assert response.status == 202


async def main(args):
    out = pathlib.Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    observations = {}
    async with async_playwright() as playwright:
        browser = await playwright.chromium.launch(executable_path=args.chromium, headless=True, args=["--no-sandbox"])
        page = await browser.new_page(viewport={"width": 1440, "height": 1000})
        errors = []
        page.on("pageerror", lambda error: errors.append(str(error)))

        async def reset():
            api(args.base, {"type": "panic"})
            api(args.base, {"type": "preset", "index": 0})
            await page.goto(args.base)
            await page.get_by_text("RUST ENGINE CONNECTED", exact=False).wait_for()
            await page.wait_for_timeout(250)

        async def set_param(index, value):
            api(args.base, {"type": "param", "id": index, "value": value})
            await page.wait_for_timeout(150)

        await reset()
        graph = page.locator("svg.envelope").first.locator("path")
        await set_param(30, .001)
        short = await graph.get_attribute("d")
        await set_param(30, 1)
        long = await graph.get_attribute("d")
        observations["attack_changes_programmed_curve"] = {
            "status": "PASS" if short != long else "FAIL",
            "attack_seconds": [.001, 1], "path_identical": short == long,
        }
        cutoff = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        wrapper = cutoff.locator("..")
        await wrapper.locator("output").click()
        numeric_count = await wrapper.locator("input[type=number]").count()
        observations["click_readout_opens_numeric_editor"] = {"status": "PASS" if numeric_count else "FAIL", "input_count": numeric_count}

        await reset()
        cutoff = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        factory = float(await cutoff.get_attribute("aria-valuenow"))
        await set_param(23, 1000)
        await cutoff.dblclick()
        await page.wait_for_timeout(150)
        after = float(await cutoff.get_attribute("aria-valuenow"))
        observations["double_click_resets_factory_value"] = {"status": "PASS" if abs(after-factory)<.01 else "FAIL", "factory": factory, "after": after}

        await reset()
        cutoff = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        before = float(await cutoff.get_attribute("aria-valuenow"))
        await cutoff.scroll_into_view_if_needed()
        rect = await cutoff.bounding_box()
        x, y = rect["x"]+rect["width"]/2, rect["y"]+rect["height"]/2
        await page.mouse.move(x, y)
        await page.mouse.down()
        await page.mouse.move(x, y-35, steps=12)
        await page.mouse.up()
        await page.wait_for_timeout(300)
        changed = float(await cutoff.get_attribute("aria-valuenow"))
        await page.get_by_role("button", name="UNDO", exact=True).click()
        await page.wait_for_timeout(250)
        undone = float(await cutoff.get_attribute("aria-valuenow"))
        observations["one_drag_is_one_undo"] = {"status": "PASS" if abs(undone-before)<.01 else "FAIL", "before": before, "changed": changed, "after_one_undo": undone}

        await reset()
        cutoff = page.get_by_role("slider", name="Filter Cutoff", exact=True)
        before = float(await cutoff.get_attribute("aria-valuenow"))
        async def delay(route):
            if route.request.post_data_json.get("type") == "param":
                await asyncio.sleep(.45)
            await route.continue_()
        await page.route("**/api/v1/command*", delay)
        await cutoff.focus()
        await cutoff.press("ArrowUp")
        optimistic = float(await cutoff.get_attribute("aria-valuenow"))
        await page.wait_for_timeout(150)
        stale = float(await cutoff.get_attribute("aria-valuenow"))
        await page.wait_for_timeout(600)
        settled = float(await cutoff.get_attribute("aria-valuenow"))
        observations["stale_ws_does_not_overwrite_edit"] = {"status": "PASS" if stale==optimistic else "FAIL", "delay_ms": 450, "before": before, "optimistic": optimistic, "during_delay": stale, "settled": settled}
        await page.unroute("**/api/v1/command*", delay)

        await reset()
        await set_param(46, 0)
        await page.route("**/api/v1/command*", delay)
        await page.get_by_role("button", name="CHORUS I", exact=True).click()
        await page.wait_for_timeout(150)
        await page.get_by_role("button", name="CHORUS II", exact=True).click()
        await page.wait_for_timeout(900)
        mode = float(await page.get_by_role("slider", name="Chorus Mode", exact=True).get_attribute("aria-valuenow"))
        observations["chorus_fast_intent_retained"] = {"status": "PASS" if mode==3 else "FAIL", "initial": 0, "expected": 3, "observed": mode, "delay_ms": 450}
        await page.unroute("**/api/v1/command*", delay)

        source = subprocess.check_output(["git", "show", BASELINE+":ui/src/App.tsx"], cwd=REPO, text=True)
        match = re.search(r"function spectrum\(samples:number\[\]\)\{.*?\}(?=\nfunction Scope)", source)
        assert match, "Original spectrum implementation not found"
        fft = match.group().replace("samples:number[]", "samples")
        benchmark = await page.evaluate("""fft => {
            const spectrum = (new Function(fft + ';return spectrum'))();
            const input = Array.from({length:1024}, (_, i) => .5*Math.sin(2*Math.PI*23*i/1024));
            for(let i=0;i<100;i++) spectrum(input);
            const times=[];
            for(let i=0;i<150;i++) {const t=performance.now();for(let j=0;j<20;j++)spectrum(input);times.push((performance.now()-t)/20);}
            times.sort((a,b)=>a-b); const q=p=>times[Math.min(times.length-1,Math.floor(p*times.length))];
            return {unit:'ms/FFT',n:1024,batches:150,per_batch:20,warmup:100,p50:q(.5),p95:q(.95),p99:q(.99),max:times.at(-1)};
        }""", fft)
        observations["legacy_js_fft"] = benchmark
        await page.screenshot(path=str(out/"baseline-desktop.png"), full_page=True)
        report = {"baseline": BASELINE, "base": args.base, "browser": browser.version, "observations": observations, "browser_errors": errors}
        (out/"baseline-probe.json").write_text(json.dumps(report, indent=2)+"\n")
        await browser.close()
    hashes = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.iterdir()) if p.is_file() and p.name != "SHA256SUMS"}
    (out/"SHA256SUMS").write_text("".join(f"{h}  {name}\n" for name,h in hashes.items()))
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="http://127.0.0.1:8787")
    parser.add_argument("--output", default="/workspace/.prophet-cloud/task-a/baseline/probe")
    parser.add_argument("--chromium", default="/usr/bin/chromium")
    asyncio.run(main(parser.parse_args()))
