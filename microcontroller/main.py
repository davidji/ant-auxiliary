#!.venv/bin/python
import client, asyncio

async def connect():
    connection = await client.connect()

    async def send():
        for i in range(10):
            connection.fan_set_duty(0.5)
            await asyncio.sleep(1)

    await asyncio.gather(connection.print_received(), send())

asyncio.run(connect())
