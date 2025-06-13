
# Extra functionality for the Ant PCB Maker

I need

 - A USB to serial adapter
 - Fan PWM control
 - OneWire temperature sensor monitoring
 - Fan and spindle RPM monitoring

I have a STM32F411 WeAct Black Pill V2, which should be able to do all that, so this project
is to write a firmware that can do the above on it.

I've gone for Rust and RTIC. When I first started messing around with embedded Rust,
I was impressed by how joined up it all was. It's a bit messier at the moment -
the splitting out of embedded-io and embedded-nb is not something the various
hals have cought up with just yet, but I suspect that's temporary.
