"""Integration tests against Agent B's actual additive protocol-v1 native host.

No v2 producer is simulated. Fault-injection cases delay/suppress real HTTP/WS
traffic in the test process only. Use a dedicated null-sink host; tests change
presets, mappings and MIDI state. This is not a physical audio-device test.
"""
import argparse
import asyncio
import hashlib
import json
import math
import pathlib
import re
import time
import traceback
import urllib.error
import urllib.request
import uuid

from playwright.async_api import async_playwright, expect
from e2e_controls import Suite


class NativeSuite(Suite):
    def __init__(self, base, output, sample_rate, assets_dir):
        super().__init__(base, output)
        self.sample_rate=sample_rate
        self.assets_dir=pathlib.Path(assets_dir)
        self.observations={}
        self.test_loaders=[]

    def api(self, command=None, expected=202):
        request=self.base+"/api/v1/state"
        if command is not None:
            request=urllib.request.Request(self.base+"/api/v1/command",data=json.dumps(command).encode(),headers={"Content-Type":"application/json"})
        try:
            with urllib.request.urlopen(request,timeout=5) as response:
                assert response.status==(200 if command is None else expected)
                result=json.load(response)
                if command is not None:assert isinstance(result.get("acceptedCommandId"),int) and result["acceptedCommandId"]>0
                return result
        except urllib.error.HTTPError as error:
            if error.code!=expected:raise
            return {"status":error.code,"body":error.read().decode()}

    async def applied(self, command, parameter=None):
        accepted=self.api(command)["acceptedCommandId"]
        if parameter is not None:
            return await self.wait_engine(lambda s:s["parameterCommandIds"][parameter]>=accepted,"Direct parameter acknowledgment did not appear")
        return await self.wait_engine(lambda s:s["control"]["lastAppliedCommandId"]==accepted,"Command application did not appear")

    async def open(self,page):
        self.api({"type":"panic"})
        await self.applied({"type":"preset","index":0},parameter=23)
        await page.goto(self.base)
        await page.get_by_text("RUST ENGINE CONNECTED",exact=False).wait_for()
        await expect(page.get_by_role("slider",name="Filter Cutoff",exact=True)).to_be_enabled()

    async def native_metadata_factory_reset(self,page):
        await self.open(page)
        with urllib.request.urlopen(self.base+"/api/v1/meta",timeout=5) as response:
            metadata=json.load(response)
        assert metadata["v"]==1
        assert {"envelope-stages","parameter-revisions","midi-learn-lifecycle","reverb-bypass","continuous-capture"}<=set(metadata["extensions"])
        assert len(metadata["parameters"])==83 and len(metadata["presets"])==10
        assert all(len(p["values"])==83 and all(math.isfinite(v) for v in p["values"]) for p in metadata["presets"])
        preset=next(p for p in metadata["presets"] if p["id"]==7)
        factory=preset["values"][23]
        await page.get_by_role("combobox",name="Factory preset",exact=True).select_option("7")
        await self.wait_engine(lambda s:abs(s["values"][23]-factory)<.01,"Native factory load did not apply")
        slider=page.get_by_role("slider",name="Filter Cutoff",exact=True)
        reset=page.get_by_role("button",name="Reset Filter Cutoff to factory preset",exact=True)
        await expect(reset).to_be_visible()
        await self.edit(page,"Filter Cutoff",1234)
        await self.wait_engine(lambda s:abs(s["values"][23]-1234)<.01,"Native numeric edit did not apply")
        await slider.dblclick()
        await self.wait_engine(lambda s:abs(s["values"][23]-factory)<.01,"Double-click did not reset actual loaded preset")
        await page.get_by_role("button",name="UNDO",exact=True).click()
        await self.wait_engine(lambda s:abs(s["values"][23]-1234)<.01,"Factory reset undo failed")
        await page.get_by_role("button",name="REDO",exact=True).click()
        await self.wait_engine(lambda s:abs(s["values"][23]-factory)<.01,"Factory reset redo failed")
        await self.edit(page,"Filter Cutoff",1390)
        await slider.focus()
        await slider.press("Alt+Enter")
        await self.wait_engine(lambda s:abs(s["values"][23]-factory)<.01,"Keyboard factory reset failed")
        await page.get_by_role("button",name="Edit Filter Cutoff",exact=True).tap()
        field=page.get_by_role("textbox",name="Filter Cutoff value",exact=True)
        await field.fill("1450")
        await field.press("Enter")
        await reset.tap()
        await self.wait_engine(lambda s:abs(s["values"][23]-factory)<.01,"Touch factory reset failed")
        self.observations["factory"]={"preset":7,"cutoff":factory,"metadataValues":83}

    async def native_acknowledgments(self,page):
        await self.open(page)
        before_enqueue,enqueue,after_enqueue,respond,finished=(asyncio.Event() for _ in range(5))
        observed={}
        async def hold(route):
            body=route.request.post_data_json
            if body.get("type")=="param" and body.get("id")==23:
                try:
                    before_enqueue.set()
                    await enqueue.wait()
                    response=await route.fetch()
                    assert response.status==202
                    observed.update(await response.json())
                    after_enqueue.set()
                    await respond.wait()
                    await route.fulfill(response=response)
                except Exception as error:observed["routeError"]=str(error)
                finally:finished.set()
            else:await route.continue_()
        await page.route("**/api/v1/command*",hold)
        slider=page.get_by_role("slider",name="Filter Cutoff",exact=True)
        try:
            await self.edit(page,"Filter Cutoff",1234)
            await asyncio.wait_for(before_enqueue.wait(),3)
            unrelated=self.api({"type":"param","id":24,"value":.31})["acceptedCommandId"]
            await self.wait_engine(lambda s:s["parameterCommandIds"][24]>=unrelated,"Unrelated external command did not apply")
            await expect(slider).to_have_attribute("aria-valuenow","1234")
            await expect(page.locator(".control-status")).to_contain_text("awaiting host")
            assert abs(self.api()["values"][23]-680)<.01,"Held command unexpectedly reached RT"
            enqueue.set()
            await asyncio.wait_for(after_enqueue.wait(),3)
            own=observed["acceptedCommandId"]
            await self.wait_engine(lambda s:s["parameterCommandIds"][23]>=own,"Own command not applied in RT")
            # An external client supersedes this same parameter before the
            # browser receives the enqueue reply. A global lastApplied ID alone
            # must never be used as a watermark for unrelated parameter edits.
            external=self.api({"type":"param","id":23,"value":1370})["acceptedCommandId"]
            await self.wait_engine(lambda s:s["parameterCommandIds"][23]==external,"External same-parameter edit did not apply")
            await page.wait_for_timeout(100)
            await expect(slider).to_have_attribute("aria-valuenow","1234")
            respond.set()
            await expect(slider).to_have_attribute("aria-valuenow","1370",timeout=3000)
            assert "routeError" not in observed,observed.get("routeError")
            self.observations["ackIds"]={"unrelated":unrelated,"own":own,"externalSuperseding":external}
        finally:
            enqueue.set();respond.set()
            if before_enqueue.is_set():await asyncio.wait_for(finished.wait(),3)
            await page.unroute("**/api/v1/command*",hold)

    async def native_202_is_not_application(self,page):
        sockets=[]
        block=False
        latest=None
        def wire(route):
            server=route.connect_to_server()
            sockets.append(route)
            def receive(message):
                nonlocal latest
                latest=message
                if not block:route.send(message)
            server.on_message(receive)
        await page.route_web_socket("**/api/v1/ws*",wire)
        await self.open(page)
        block=True
        async with page.expect_response(lambda r:"/api/v1/command" in r.url and r.request.post_data_json.get("type")=="param") as response_info:
            await self.edit(page,"Filter Cutoff",1500)
        response=await response_info.value
        accepted=(await response.json())["acceptedCommandId"]
        await self.wait_engine(lambda s:s["parameterCommandIds"][23]>=accepted,"RT never applied accepted command")
        await page.wait_for_timeout(200)
        await expect(page.locator(".control-status")).to_contain_text("awaiting host")
        assert latest is not None
        block=False
        sockets[-1].send(latest)
        await expect(page.locator(".control-status")).not_to_contain_text("awaiting host",timeout=3000)

    async def native_midi_lifecycle(self,page):
        await self.open(page)
        for parameter in (23,24):await self.applied({"type":"unlearn","id":parameter})
        cutoff=page.get_by_role("slider",name="Filter Cutoff",exact=True)
        await cutoff.click(button="right")
        dialog=page.get_by_role("dialog",name="MIDI Learn",exact=True)
        await dialog.get_by_role("button",name="Start MIDI Learn",exact=True).click()
        await self.wait_engine(lambda s:s["midiLearn"]["active"] and s["midiLearn"]["target"]==23,"Native Learn target not armed")
        await dialog.get_by_role("button",name="Cancel MIDI Learn",exact=True).click()
        await self.wait_engine(lambda s:not s["midiLearn"]["active"],"Native cancel did not stop capture")
        self.api({"type":"midi","status":176,"data1":16,"data2":64})
        await page.wait_for_timeout(100)
        assert self.api()["midiLearn"]["ccMap"][16] is None,"Cancelled Learn still captured a CC"
        await dialog.get_by_role("button",name="Start MIDI Learn",exact=True).click()
        await self.wait_engine(lambda s:s["midiLearn"]["active"],"Learn not restarted")
        self.api({"type":"midi","status":176,"data1":16,"data2":64})
        await self.wait_engine(lambda s:not s["midiLearn"]["active"] and s["midiLearn"]["ccMap"][16]==23,"Capture result absent from real ccMap")
        await expect(dialog).to_contain_text("16")
        cc=dialog.get_by_role("spinbutton",name="MIDI mapping CC",exact=True)
        await cc.fill("17")
        await dialog.get_by_role("button",name="Replace MIDI mapping",exact=True).click()
        await self.wait_engine(lambda s:s["midiLearn"]["ccMap"][16] is None and s["midiLearn"]["ccMap"][17]==23,"Remap did not clear prior parameter CC")
        await dialog.get_by_role("button",name="Dismiss MIDI Learn panel",exact=True).click()
        await page.get_by_role("slider",name="Filter Resonance",exact=True).click(button="right")
        await dialog.get_by_role("spinbutton",name="MIDI mapping CC",exact=True).fill("17")
        await dialog.get_by_role("button",name="Replace MIDI mapping",exact=True).click()
        await self.wait_engine(lambda s:s["midiLearn"]["ccMap"][17]==24,"Conflicting CC did not transfer to new target")
        assert 23 not in self.api()["midiLearn"]["ccMap"],"Old target retained binding after explicit conflict replacement"
        before=list(self.api()["midiLearn"]["ccMap"])
        for reserved in (64,120,123):
            await dialog.get_by_role("spinbutton",name="MIDI mapping CC",exact=True).fill(str(reserved))
            await dialog.get_by_role("button",name="Replace MIDI mapping",exact=True).click()
            await expect(page.locator(".control-status")).to_contain_text("reserved")
            assert self.api()["midiLearn"]["ccMap"]==before,"UI reserved-CC attempt mutated mappings"
            rejected=self.api({"type":"remap","cc":reserved,"id":24},expected=400)
            assert rejected["status"]==400 and self.api()["midiLearn"]["ccMap"]==before
        await expect(dialog).to_contain_text("Mappings are global across channels")
        assert await dialog.get_by_label("MIDI mapping channel",exact=True).count()==0,"Native UI invented channel-specific mappings"
        # Maps are global across channels. Do not show fictitious per-channel
        # assignments; CC74 has separate MPE member-channel precedence.
        for channel,value in ((0,31),(2,93)):
            self.api({"type":"midi","status":176+channel,"data1":17,"data2":value})
            await self.wait_engine(lambda s:abs(s["values"][24]-value/127)<1e-5,"Global CC map not applied across channels")
        await dialog.get_by_role("button",name="Remove MIDI mapping",exact=True).click()
        await self.wait_engine(lambda s:24 not in s["midiLearn"]["ccMap"],"Unlearn did not remove parameter mappings")
        await self.applied({"type":"learn","id":23})
        self.api({"type":"midi","status":177,"data1":74,"data2":64})
        await page.wait_for_timeout(100)
        assert self.api()["midiLearn"]["active"],"MPE member-channel CC74 improperly completed Learn"
        self.api({"type":"midi","status":176,"data1":74,"data2":64})
        await self.wait_engine(lambda s:not s["midiLearn"]["active"] and s["midiLearn"]["ccMap"][74]==23,"Master-channel CC74 failed Learn")
        await self.applied({"type":"unlearn","id":23})
        self.observations["midi"]={"reservedRejected":[64,120,123],"globalChannelsTested":[0,2],"mpeMember74DoesNotCapture":True}

    async def native_envelopes(self,page):
        await self.open(page)
        assert self.api()["sampleRate"]==self.sample_rate
        await self.applied({"type":"reverb_bypass","enabled":False,"tail":"kill"})
        for parameter,value in ((63,1),(66,1),(43,12),(30,2),(31,1),(32,.5),(33,.5),(34,2),(35,1),(36,.5),(37,.5)):
            await self.applied({"type":"param","id":parameter,"value":value},parameter=parameter)
        self.api({"type":"midi","status":144,"data1":60,"data2":100})
        first=await self.wait_engine(lambda s:s["activeVoiceMask"].bit_count()==12 and all(v["stage"]==1 and v["timed"] for v in s["ampEnvelopes"]),"12 voices did not report actual Attack stages")
        await page.wait_for_timeout(150)
        second=self.api()
        assert len(second["ampEnvelopes"])==len(second["filterEnvelopes"])==12
        a,b=first["ampEnvelopes"][0],second["ampEnvelopes"][0]
        assert b["stage"]==1 and 0<=a["progress"]<b["progress"]<=1
        assert abs((b["elapsedSamples"]-a["elapsedSamples"])-(second["frames"]-first["frames"]))<=256,"Envelope counters did not advance at base sample rate"
        running=page.get_by_role("list",name="Running amp envelopes",exact=True)
        await expect(running.get_by_role("listitem")).to_have_count(12)
        await expect(running.get_by_role("listitem").first).to_contain_text("Attack")
        await expect(running.get_by_role("listitem").first).to_contain_text("%")
        self.api({"type":"midi","status":144,"data1":60,"data2":100})
        retrigger=await self.wait_engine(lambda s:s["ampEnvelopes"][0]["stage"]==1 and s["ampEnvelopes"][0]["elapsedSamples"]<b["elapsedSamples"],"Retrigger stage time did not reset")
        self.api({"type":"midi","status":128,"data1":60,"data2":0})
        released=await self.wait_engine(lambda s:all(v["stage"]==4 for v in s["ampEnvelopes"]),"NoteOff did not report Release")
        assert all(v["timed"] and 0<=v["progress"]<=1 for v in released["ampEnvelopes"])
        idle=await self.wait_engine(lambda s:s["activeVoiceMask"]==0 and all(v["stage"]==0 for v in s["ampEnvelopes"]),"Amp voices did not become Idle")
        assert all(not v["timed"] and v["progress"]==0 for v in idle["ampEnvelopes"])
        self.observations["envelopes"]={"sampleRate":self.sample_rate,"voices":12,"elapsedDelta":b["elapsedSamples"]-a["elapsedSamples"],"frameDelta":second["frames"]-first["frames"],"retriggerElapsed":retrigger["ampEnvelopes"][0]["elapsedSamples"],"actualFields":["stage","elapsedSamples","level","progress","timed"]}

    async def native_reverb(self,page):
        await self.open(page)
        await self.applied({"type":"reverb_bypass","enabled":True,"tail":"spill"})
        for parameter,value in ((51,0),(53,.5),(54,0),(62,.5),(37,.1)):
            await self.applied({"type":"param","id":parameter,"value":value},parameter=parameter)
        self.api({"type":"midi","status":144,"data1":60,"data2":100})
        await self.wait_engine(lambda s:not s["reverb"]["dormant"] and max(abs(x) for x in s["scope"])>.001,"Reverb tank never became active")
        await page.wait_for_timeout(300)
        self.api({"type":"midi","status":128,"data1":60,"data2":0})
        await page.get_by_role("button",name="Reverb ON",exact=True).click()
        spill=await self.wait_engine(lambda s:not s["reverb"]["enabled"] and s["reverb"]["tail"]=="spill" and s["reverb"]["enableGain"]==0,"Spill bypass not applied")
        assert not spill["reverb"]["dormant"],"Spill unexpectedly killed an excited tail"
        await page.get_by_role("combobox",name="Reverb tail policy",exact=True).select_option("kill")
        killed=await self.wait_engine(lambda s:s["reverb"]["tail"]=="kill" and s["reverb"]["dormant"],"Kill did not enter dormancy")
        await page.get_by_role("button",name="Reverb BYPASSED",exact=True).click()
        enabled=await self.wait_engine(lambda s:s["reverb"]["enabled"] and s["reverb"]["enableGain"]==1,"Re-enable ramp did not complete")
        self.observations["reverb"]={"spill":spill["reverb"],"kill":killed["reverb"],"enabled":enabled["reverb"],"scope":"actual state transitions; not calibrated RT60 or audible-device qualification"}

    async def native_capture(self,page):
        await self.open(page)
        self.api({"type":"midi","status":144,"data1":60,"data2":100})
        await self.wait_engine(lambda s:max(abs(x) for x in s["scope"])>.001,"No real PCM excitation")
        dropped={"count":0}
        def wire(route):
            server=route.connect_to_server()
            messages=0
            def receive(message):
                nonlocal messages
                messages+=1
                if messages==4:dropped["count"]+=1;return
                route.send(message)
            server.on_message(receive)
        await page.route_web_socket("**/api/v1/capture*",wire)
        result=await page.evaluate("""() => new Promise((resolve,reject)=>{
            const socket=new WebSocket(`${location.protocol==='https:'?'wss:':'ws:'}//${location.host}/api/v1/capture`);
            const blocks=[];let messages=0;
            const timeout=setTimeout(()=>{socket.close();reject(Error('PCM capture timeout'))},5000);
            socket.onerror=()=>{clearTimeout(timeout);reject(Error('PCM WebSocket error'))};
            socket.onmessage=event=>{
                try{const data=JSON.parse(event.data);if(data.v!==1||!Array.isArray(data.blocks))throw Error('Invalid capture message');blocks.push(...data.blocks);messages++;
                    if(messages>=24){clearTimeout(timeout);socket.close();resolve({messages,blocks})}
                }catch(error){clearTimeout(timeout);socket.close();reject(error)}
            };
        })""")
        blocks=result["blocks"]
        assert dropped["count"]>=1 and len(blocks)>10
        gaps=0;nonzero=0;previous=None
        for block in blocks:
            assert isinstance(block["sequence"],int) and isinstance(block["firstFrame"],int)
            assert block["sampleRate"]==self.sample_rate and 0<len(block["samples"])<=64
            assert all(math.isfinite(value) for value in block["samples"])
            nonzero+=sum(abs(value)>.001 for value in block["samples"])
            if previous and (block["sequence"]!=previous["sequence"]+1 or block["firstFrame"]!=previous["firstFrame"]+len(previous["samples"])):gaps+=1
            previous=block
        assert gaps>=1 and nonzero>0,"Dropped real PCM message was not detectable or stream was silent"
        self.observations["capture"]={"sampleRate":self.sample_rate,"messages":result["messages"],"blocks":len(blocks),"gapCount":gaps,"testSuppressedMessages":dropped["count"],"nonzeroSamples":nonzero,"maxChunkSamples":max(len(b["samples"]) for b in blocks),"capturedBlocksSha256":hashlib.sha256(json.dumps(blocks,separators=(",",":")).encode()).hexdigest(),"scope":"raw real PCM transport acceptance; UI/WASM STFT reset checked separately when integrated"}

    async def native_spectrum(self,page):
        # Test-only worker instrumentation drops one genuine capture message and
        # closes its genuine socket. No PCM samples, sequence or frequency data
        # are generated/replaced by this instrumentation.
        prefix="""
globalThis.__nativeFault={drop:0,dropped:0,gapEnd:0,connections:0};
const __messageDescriptor=Object.getOwnPropertyDescriptor(WebSocket.prototype,'onmessage');
Object.defineProperty(WebSocket.prototype,'onmessage',{
  configurable:true,enumerable:__messageDescriptor.enumerable,get:__messageDescriptor.get,
  set(handler){
    if(!this.url.includes('/api/v1/capture'))return __messageDescriptor.set.call(this,handler);
    globalThis.__nativeCaptureSocket=this;globalThis.__nativeFault.connections++;
    __messageDescriptor.set.call(this,event=>{const fault=globalThis.__nativeFault;if(fault.drop>0){fault.drop--;fault.dropped++;try{const blocks=JSON.parse(event.data).blocks;const last=blocks.at(-1);fault.gapEnd=last.firstFrame+last.samples.length;}catch{}return;}handler?.call(this,event);});
  }
});
"""
        # Chromium classifies fulfilled/Blob worker scripts differently for
        # loopback WebSockets. Serve the test loader as a genuine same-origin
        # network file, importing the untouched production worker module.
        assets=self.assets_dir
        production=max(assets.glob('spectrum.worker-*.js'),key=lambda p:p.stat().st_mtime_ns)
        loader=assets/('pcm-test-loader-'+uuid.uuid4().hex+'.mjs')
        loader.write_text(prefix+"""
const __startupMessages=[];
const __queueStartup=event=>{__startupMessages.push(event.data);event.stopImmediatePropagation();};
self.addEventListener('message',__queueStartup);
await import("""+json.dumps('./'+production.name)+""");
self.removeEventListener('message',__queueStartup);
for(const data of __startupMessages)self.dispatchEvent(new MessageEvent('message',{data}));
""")
        self.test_loaders.append(loader)
        await page.add_init_script("""(() => {
          const NativeURL=globalThis.URL;
          const loader="""+json.dumps('/assets/'+loader.name)+""";
          function InstrumentedURL(url,base){
            if(String(url).includes('spectrum.worker')){
              return new NativeURL(loader,base||location.href);
            }
            return new NativeURL(url,base);
          }
          Object.setPrototypeOf(InstrumentedURL,NativeURL);
          InstrumentedURL.prototype=NativeURL.prototype;
          globalThis.URL=InstrumentedURL;
        })();""")
        workers=[]
        page.on("worker",lambda worker:workers.append(worker))
        await self.open(page)
        for parameter,value in ((0,0),(1,0),(3,1),(4,0),(5,0),(14,.5),(15,0),(16,0),(18,0),(19,0),(23,18000),(24,0),(25,.25),(26,0),(28,0),(34,.001),(35,.001),(36,1),(37,.05),(41,0),(46,0),(47,0),(63,1),(64,.5),(66,0)):
            await self.applied({"type":"param","id":parameter,"value":value},parameter=parameter)
        await self.applied({"type":"reverb_bypass","enabled":False,"tail":"kill"})
        self.api({"type":"midi","status":144,"data1":57,"data2":100})
        measurement=page.get_by_test_id("spectrum-measurement")
        await expect(measurement).to_be_visible(timeout=10000)
        assert workers,"Worker fault-injection hook did not attach"
        worker=workers[-1]
        assert await worker.evaluate("Boolean(globalThis.__nativeFault)"),"Worker was not instrumented; cannot claim gap test"

        async def reading(target,size,window):
            await page.wait_for_function("""({target,size,rate,window})=>{
                const el=document.querySelector('[data-testid=spectrum-measurement]');
                if(!el)return false;const m=el.textContent.match(/Peak ([0-9.]+) Hz.*?(-?[0-9.]+) dBFS/);
                return m&&Math.abs(Number(m[1])-target)<=rate/size*1.1&&Number(m[2])>-80&&Number(el.dataset.endFrame)-Number(el.dataset.firstFrame)===size&&Array.from(document.querySelectorAll('[role=status]')).some(s=>s.textContent.includes(size+'-point '+window)&&s.textContent.includes(rate+' Hz'));
            }""",arg={"target":target,"size":size,"rate":self.sample_rate,"window":window},timeout=10000)
            text=await measurement.inner_text()
            match=re.search(r"Peak ([0-9.]+) Hz.*?(-?[0-9.]+) dBFS",text)
            return {"targetHz":target,"peakHz":float(match.group(1)),"peakDbfs":float(match.group(2)),"fftSize":size,"window":window,"firstFrame":int(await measurement.get_attribute("data-first-frame")),"endFrame":int(await measurement.get_attribute("data-end-frame")),"discontinuities":int(await measurement.get_attribute("data-discontinuities"))}

        readings=[await reading(220,4096,"hann")]
        await page.get_by_role("combobox",name="FFT size",exact=True).select_option("8192")
        await page.get_by_role("combobox",name="Spectrum window",exact=True).select_option("blackman-harris")
        await page.get_by_role("spinbutton",name="Spectrum average alpha",exact=True).fill("1")
        await page.get_by_role("spinbutton",name="Spectrum peak hold frames",exact=True).fill("5")
        await page.get_by_role("spinbutton",name="Spectrum peak decay",exact=True).fill("0.5")
        readings.append(await reading(220,8192,"blackman-harris"))
        self.api({"type":"midi","status":128,"data1":57,"data2":0})
        self.api({"type":"midi","status":144,"data1":69,"data2":100})
        readings.append(await reading(440,8192,"blackman-harris"))
        await page.get_by_role("combobox",name="FFT size",exact=True).select_option("2048")
        await page.get_by_role("combobox",name="Spectrum window",exact=True).select_option("kaiser")
        await page.get_by_role("spinbutton",name="Kaiser beta",exact=True).fill("6")
        readings.append(await reading(440,2048,"kaiser"))

        before=int(await measurement.get_attribute("data-discontinuities"))
        await worker.evaluate("globalThis.__nativeFault.drop=1")
        deadline=time.monotonic()+5
        while not await worker.evaluate("globalThis.__nativeFault.dropped>=1"):
            assert time.monotonic()<deadline,"No real capture packet was suppressed"
            await asyncio.sleep(.05)
        fault=await worker.evaluate("globalThis.__nativeFault")
        await page.wait_for_function("""({before,gapEnd})=>{const e=document.querySelector('[data-testid=spectrum-measurement]');return e&&Number(e.dataset.discontinuities)>before&&Number(e.dataset.firstFrame)>=gapEnd;}""",arg={"before":before,"gapEnd":fault["gapEnd"]},timeout=10000)
        gap=await reading(440,2048,"kaiser")
        assert gap["firstFrame"]>=fault["gapEnd"] and gap["endFrame"]-gap["firstFrame"]==2048,"STFT window spanned an injected real PCM gap"
        old_end=gap["endFrame"];old_resets=gap["discontinuities"]
        await worker.evaluate("globalThis.__nativeCaptureSocket.close(4001,'test-only PCM reconnect')")
        await page.get_by_text("PCM disconnected; retrying with cleared overlap",exact=True).wait_for(timeout=3000)
        await page.wait_for_function("""({end,resets})=>{const e=document.querySelector('[data-testid=spectrum-measurement]');return e&&Number(e.dataset.firstFrame)>end&&Number(e.dataset.discontinuities)>resets;}""",arg={"end":old_end,"resets":old_resets},timeout=10000)
        reconnected=await reading(440,2048,"kaiser")
        connections=await worker.evaluate("globalThis.__nativeFault.connections")
        assert connections>=2 and reconnected["firstFrame"]>old_end
        self.observations["spectrum"]={"sampleRate":self.sample_rate,"readings":readings,"injectedDrop":fault,"freshWindowAfterGap":gap,"freshWindowAfterReconnect":reconnected,"realCaptureConnections":connections,"scope":"actual native PCM -> Rust WASM STFT in dedicated browser worker -> rendered measurement; test only drops one real packet/closes real socket"}

    async def native_transfer(self,page):
        first,release=asyncio.Event(),asyncio.Event()
        calls=[];active=0;maximum=0
        async def delayed(route):
            nonlocal active,maximum
            active+=1;maximum=max(maximum,active)
            values=route.request.post_data_json
            calls.append(values[23])
            try:
                if len(calls)==1:
                    first.set()
                    await release.wait()
                response=await route.fetch()
                await route.fulfill(response=response)
            finally:active-=1
        await page.route("**/api/v1/filter*",delayed)
        await self.open(page)
        await asyncio.wait_for(first.wait(),5)
        try:
            for value in (1000,1100,1200,1300,1400,1500,1600,1700,1800,1900):
                await self.edit(page,"Filter Cutoff",value)
                await page.wait_for_timeout(25)
            await page.wait_for_timeout(220)
            assert len(calls)==1,"Superseded transfer intents spawned concurrent requests"
            release.set()
            await expect(page.locator(".filterchart [role=status]")).to_have_text("Small-signal transfer response",timeout=10000)
            assert maximum==1 and len(calls)==2 and calls[-1]==1900,{"maxConcurrent":maximum,"calls":calls}
            values=self.api()["values"]
            request=urllib.request.Request(self.base+"/api/v1/filter",data=json.dumps(values).encode(),headers={"Content-Type":"application/json"})
            with urllib.request.urlopen(request,timeout=10) as response:data=json.load(response)
            assert data["sampleRate"]==self.sample_rate,{"expectedSampleRate":self.sample_rate,"actual":data["sampleRate"]}
            assert data["fftSize"]==data["impulseSamples"]*2 and data["impulseSamples"]>=4096
            assert data["tailConverged"] is True
            await expect(page.locator(".filterchart")).to_contain_text(f"{self.sample_rate} Hz · {data['fftSize']}-point FFT ({data['impulseSamples']} samples) · Δf {data['binHz']:.3f} Hz · tail converged")
            path=await page.get_by_role("img",name="VCF magnitude and unwrapped phase",exact=True).locator("path").nth(1).get_attribute("d")
            points=re.findall(r"([ML])(-?[0-9.]+),(-?[0-9.]+)",path)
            expected=[];drawing=False
            for hz,db,phase in zip(data["hz"],data["db"],data["phase"]):
                if phase is None or db < -100:drawing=False;continue
                expected.append(("L" if drawing else "M",500*math.log(hz/data["hz"][0])/math.log(data["hz"][-1]/data["hz"][0]),75-phase*12))
                drawing=True
            assert len(points)==len(expected)
            for actual,reference in zip(points,expected):
                assert actual[0]==reference[0] and abs(float(actual[1])-reference[1])<.002 and abs(float(actual[2])-reference[2])<.002,"VCF graph differs from actual FFT transfer unwrap/mask"
            for left,right in zip(points,points[1:]):
                if right[0]=="L":assert abs(float(right[2])-float(left[2]))<=math.pi*12+.002,"Wrapped phase branch jump drawn as contiguous line"
            self.observations["transfer"]={"audioSampleRate":self.sample_rate,"measurementSampleRate":data["sampleRate"],"fftSize":data["fftSize"],"impulseSamples":data["impulseSamples"],"tailConverged":data["tailConverged"],"tailRelativeRms":data["tailRelativeRms"],"binHz":data["binHz"],"maxConcurrent":maximum,"requestsDuringTenEdits":len(calls),"cutoffs":calls,"visiblePhasePoints":len(points),"maskedPhasePoints":len(data["db"])-len(points),"pixelTolerance":.002,"scope":"actual prophet-analysis TransferAnalyzer impulse-response FFT at host sample rate with adaptive tail convergence"}
        finally:
            release.set()
            # Let any in-flight route finish before removing its handler.
            deadline=time.monotonic()+5
            while active and time.monotonic()<deadline:await asyncio.sleep(.02)
            await page.unroute("**/api/v1/filter*",delayed)

    async def native_watchdog(self,page):
        sockets=[];withhold=False
        def wire(route):
            sockets.append(route)
            connection=len(sockets)
            server=route.connect_to_server()
            server.on_message(lambda message:route.send(message) if not (withhold and connection==1) else None)
        await page.route_web_socket("**/api/v1/ws*",wire)
        await self.open(page)
        before=self.api()["frames"]
        withhold=True
        slider=page.get_by_role("slider",name="Filter Cutoff",exact=True)
        await expect(slider).to_be_disabled(timeout=6500)
        assert self.api()["frames"]>before,"Backend itself stalled; this was not a telemetry-delivery watchdog test"
        await expect(slider).to_be_enabled(timeout=8000)
        assert len(sockets)>=2,"Stale telemetry did not trigger a fresh connection"
        self.observations["watchdog"]={"connections":len(sockets),"producerFramesBefore":before,"producerFramesAfter":self.api()["frames"],"scope":"only delivery of genuine first-connection state snapshots withheld; backend kept advancing"}

    async def run(self,chromium,selected=None):
        self.output.mkdir(parents=True,exist_ok=True)
        cases=[self.numeric,self.drag_undo,self.touch_keyboard,self.delayed_controls,self.dependencies,self.native_metadata_factory_reset,self.native_acknowledgments,self.native_202_is_not_application,self.native_midi_lifecycle,self.native_envelopes,self.native_reverb,self.reconnect,self.native_capture,self.native_spectrum,self.native_transfer,self.native_watchdog]
        if selected:
            unknown=set(selected)-{case.__name__ for case in cases}
            if unknown:raise ValueError("Unknown cases: "+", ".join(sorted(unknown)))
            cases=[case for case in cases if case.__name__ in selected]
        async with async_playwright() as playwright:
            browser=await playwright.chromium.launch(executable_path=chromium,headless=True,args=["--no-sandbox"])
            for case in cases:
                context=await browser.new_context(viewport={"width":1440,"height":1000},has_touch=True)
                page=await context.new_page();errors=[]
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
                for loader in self.test_loaders:loader.unlink(missing_ok=True)
                self.test_loaders=[]
                self.api({"type":"panic"})
                print(json.dumps(result),flush=True)
            await browser.close()
        report={"backend":"actual Agent B native additive protocol v1; no speculative v2 fixture","backendRef":"211c4f4","base":self.base,"sampleRate":self.sample_rate,"results":self.results,"observations":self.observations,"status":"FAIL" if any(r["status"]=="FAIL" for r in self.results) else "PASS"}
        (self.output/"e2e-native.json").write_text(json.dumps(report,indent=2)+"\n")
        files=sorted(p for p in self.output.iterdir() if p.is_file() and p.name!="SHA256SUMS")
        (self.output/"SHA256SUMS").write_text("".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n" for p in files))
        return int(report["status"]!="PASS")


if __name__=="__main__":
    default_assets=pathlib.Path(__file__).resolve().parents[1]/"dist"/"assets"
    parser=argparse.ArgumentParser()
    parser.add_argument("--base",default="http://127.0.0.1:8792")
    parser.add_argument("--output",default="/tmp/prophet-e2e-native")
    parser.add_argument("--sample-rate",type=int,default=48000)
    parser.add_argument("--assets",default=str(default_assets))
    parser.add_argument("--chromium",default="/usr/bin/chromium")
    parser.add_argument("--case",action="append")
    args=parser.parse_args()
    raise SystemExit(asyncio.run(NativeSuite(args.base,args.output,args.sample_rate,args.assets).run(args.chromium,args.case)))
