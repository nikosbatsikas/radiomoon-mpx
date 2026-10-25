# Screenshots of the web pages of the headless editions for the wiki (Playwright).
#   python web_shots.py OUTDIR PASSWORD
import sys, asyncio
from playwright.async_api import async_playwright

OUT, PW = sys.argv[1], sys.argv[2]

async def shoot(p, port, name, tabs):
    b = await p.chromium.launch()
    pg = await b.new_page(viewport={"width": 1000, "height": 900}, device_scale_factor=1)
    await pg.goto(f"http://127.0.0.1:{port}/")
    await pg.wait_for_selector("#lp")
    await pg.screenshot(path=f"{OUT}/web-{name}-login.png")
    await pg.fill("#lp", PW)
    await pg.click("#lb")
    await pg.wait_for_selector("nav button")
    await pg.wait_for_timeout(3000)
    for label, slug in tabs:
        await pg.click(f'nav button:has-text("{label}")')
        await pg.wait_for_timeout(1500)
        await pg.screenshot(path=f"{OUT}/web-{name}-{slug}.png", full_page=True)
    await pg.click("#aboutbtn")
    await pg.wait_for_timeout(800)
    await pg.screenshot(path=f"{OUT}/web-{name}-about.png")
    await b.close()

async def main():
    async with async_playwright() as p:
        # the receiver first (its SFN has locked by then), the transmitter after
        await shoot(p, 8081, "receiver", [("Status", "status"), ("Log", "log"), ("Input & protection", "input"), ("Output", "output"), ("SFN", "sfn"), ("Backup", "backup"), ("Login", "login-tab")])
        await shoot(p, 8080, "transmitter", [("Status", "status"), ("Audio source", "source"), ("IP output", "ip"), ("Sound card output", "card"), ("Signal", "signal"), ("RDS", "rds"), ("SFN", "sfn"), ("Login", "login-tab")])

asyncio.run(main())
