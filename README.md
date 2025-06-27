
# Extra functionality for the Ant PCB Maker

I need

 - A USB to serial adapter
 - Fan PWM control
 - DHT11 temperature sensor monitoring
 - Fan and spindle RPM monitoring

I have a STM32F411 WeAct Black Pill V2, which should be able to do all that.

The firmware is built in Rust and RTIC, so the various tasks can run independently.
To avoid designing a protocol that combines serial with RPC, the interface is
TCP/IP - with separate serial and RPC services.

## DHT11

The DHT11 turns out to be complicated: there are existing client libraries, but they contain a critical section that lasts for the whole reading, of about 4ms. This project includes an RTIC based
client that handles EXTI interrupts, and uses the monotonic timer to timestamp the interrupts,
calculating the value from intervals between the timestamps.

The Hardware is actually capable of capturing the clock value on a falling edge and
using DMA to do so repeatedly, so there's no need to handle an interrupt for each falling
edge. The HAL doesn't support that though, and this works reliably with sufficiently low
overhead, provided you turn on optimisation.

## Regenerating protobuf/python

