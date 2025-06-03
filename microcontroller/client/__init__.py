from . import aux_pb2 as aux
from cobs import cobs
import asyncio

class AuxClient:
    def __init__(self, 
                 reader: asyncio.StreamReader, 
                 writer: asyncio.StreamWriter):
        self.reader = reader
        self.writer = writer

    async def recv(self) -> aux.Response:
        response = aux.Response()
        response.ParseFromString(cobs.decode((await self.reader.readuntil(b'\0'))[:-1]))
        return response

    def send(self, message: aux.Request):
        message = cobs.encode(message.SerializeToString()) + b'\0'
        print("message length: ", len(message))
        self.writer.write(message)
    
    async def print_received(self):
        while not self.reader.at_eof():
            message = await self.recv()
            print(self.protobuf_to_json(message))

    def fan_set_duty(self, duty: float):
        request = aux.Request()
        request.fan.set.duty = int(0xffff*duty)
        self.send(request)

    @staticmethod
    def protobuf_to_json(message):
        from google.protobuf.json_format import MessageToJson
        return MessageToJson(message, indent=2)

async def connect() -> AuxClient:
    (reader, writer) = await asyncio.open_connection("10.0.0.1", 1338)
    return AuxClient(reader, writer)
