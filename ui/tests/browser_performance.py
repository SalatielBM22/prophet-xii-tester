"""Measure browser main-thread responsiveness during one continuous knob gesture.

This is an interaction/paint/transport measurement, not an isolated React render
or audio-callback benchmark. Use identical settings for before/after runs.
"""
import argparse
import asyncio
import hashlib
import json
import pathlib
import urllib.request

from playwright.async_api import async_playwright


async def main(args):
    output=pathlib.Path(args.output)
    output.mkdir(parents=True,exist_ok=True)
    async with async_playwright() as playwright:
        browser=await playwright.chromium.launch(executable_path=args.chromium,headless=True,args=["--no-sandbox"])
        page=await browser.new_page(viewport={"width":1440,"height":1000})
        errors=[]
        page.on("pageerror",lambda error:errors.append(str(error)))
        request=urllib.request.Request(args.base+"/api/v1/command",data=b'{"type":"preset","index":0}',headers={"Content-Type":"application/json"})
        urllib.request.urlopen(request,timeout=5).close()
        await page.goto(args.base)
        await page.get_by_text("RUST ENGINE CONNECTED",exact=False).wait_for()
        slider=page.get_by_role("slider",name="Filter Cutoff",exact=True)
        await slider.scroll_into_view_if_needed()
        cdp=await page.context.new_cdp_session(page)
        await cdp.send("Performance.enable")
        before=await cdp.send("Performance.getMetrics")
        response=await slider.evaluate("""async (slider,seconds) => {
            const tasks=[],intervals=[];
            const observer=new PerformanceObserver(list=>tasks.push(...list.getEntries().map(e=>e.duration)));
            observer.observe({type:'longtask',buffered:false});
            const rect=slider.getBoundingClientRect();
            const x=rect.x+rect.width/2,y=rect.y+rect.height/2;
            const ev=(type,dy)=>slider.dispatchEvent(new PointerEvent(type,{bubbles:true,pointerId:1,pointerType:'mouse',button:0,buttons:type==='pointerup'?0:1,clientX:x,clientY:y-dy}));
            // Dispatch a single complete gesture. This stress-tests React handlers,
            // drawing, state reconciliation and command coalescing, not browser hit testing.
            ev('pointerdown',0);
            let frames=0,start=performance.now(),last=start;
            await new Promise(resolve=>{
                function frame(now){
                    if(frames) intervals.push(now-last);
                    last=now;
                    ev('pointermove',20+15*Math.sin(frames/8));
                    frames++;
                    if(now-start<seconds*1000) requestAnimationFrame(frame); else resolve();
                }
                requestAnimationFrame(frame);
            });
            ev('pointerup',0);
            observer.disconnect();
            intervals.sort((a,b)=>a-b);
            const q=p=>intervals[Math.min(intervals.length-1,Math.floor(p*intervals.length))];
            return {frames,elapsedMs:performance.now()-start,frameIntervalMs:{p50:q(.5),p95:q(.95),p99:q(.99),max:intervals.at(-1)},longTasks:tasks.length,longTaskMaxMs:Math.max(0,...tasks)};
        }""",args.seconds)
        after=await cdp.send("Performance.getMetrics")
        metrics=lambda result:{x["name"]:x["value"] for x in result["metrics"]}
        pre,post=metrics(before),metrics(after)
        response["cpuSeconds"]={k:post.get(k,0)-pre.get(k,0) for k in ["ScriptDuration","LayoutDuration","RecalcStyleDuration","TaskDuration"]}
        response["heapBytesBeforeAfter"]=[pre.get("JSHeapUsedSize"),post.get("JSHeapUsedSize")]
        response["scope"]="browser interaction/paint/transport; not isolated React commits or audio callback"
        response["label"],response["browser"],response["errors"]=args.label,browser.version,errors
        response["p95BudgetMs"]=args.p95_budget_ms
        response["status"]="PASS" if not errors and response["frames"]>=30 and response["frameIntervalMs"]["p95"]<=args.p95_budget_ms else "FAIL"
        (output/(args.label+"-browser-performance.json")).write_text(json.dumps(response,indent=2)+"\n")
        file=output/(args.label+"-browser-performance.json")
        (output/(args.label+"-SHA256SUMS")).write_text(hashlib.sha256(file.read_bytes()).hexdigest()+"  "+file.name+"\n")
        await cdp.detach()
        await browser.close()
        print(json.dumps(response,indent=2))
        return int(response["status"]!="PASS")


if __name__=="__main__":
    parser=argparse.ArgumentParser()
    parser.add_argument("--base",default="http://127.0.0.1:8787")
    parser.add_argument("--output",default="/workspace/.prophet-cloud/task-a/performance")
    parser.add_argument("--label",default="current")
    parser.add_argument("--seconds",type=float,default=6)
    parser.add_argument("--p95-budget-ms",type=float,default=50)
    parser.add_argument("--chromium",default="/usr/bin/chromium")
    args=parser.parse_args()
    if args.seconds<1 or args.seconds>30:parser.error("seconds must be between 1 and 30")
    raise SystemExit(asyncio.run(main(args)))
