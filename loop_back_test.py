#!/usr/bin/env python3
import serial, os, io

def loop_back_test(ser, iterations=100, length=256):
    reader = io.BufferedReader(ser, buffer_size=length)
    writer = io.BufferedWriter(ser, buffer_size=length)
    for iteration in range(iterations):
        print("iteration: {}".format(iteration))
        sent = os.urandom(int(length/2)).hex().encode()
        writer.write(sent)
        writer.flush()
        received = reader.read(length)
        if sent != received:
            print("sent and received differ: {}, {}".format(sent.decode(), received.decode()))

ser = serial.Serial('/dev/ttyACM1', 115200, 8, "N", 1)
loop_back_test(ser)
ser.close()
