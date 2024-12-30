import serial as ser

serial = ser.Serial("/dev/ttyACM0", 115200)

serial.write(b"connect\n")

while True:
    data = serial.read(1)
    print(data.hex())

