
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
client that handles EXT interrupts, and uses the monotonic timer to timestamp the interrupts,
calculating the value from intervals between the timestamps.

The Hardware is actually capable of capturing the clock value on a falling edge and
using DMA to do so repeatedly, so there's no need to handle an interrupt for each falling
edge. The HAL doesn't support that though, and this works reliably with sufficiently low
overhead, provided you turn on optimisation.

## Regenerating protobuf/python

## Networking

The device uses DHCP to get it's IP address. The idea was that the host would bridge
it onto the network, so it would be available network wide, without any routing.
That can't work for Wifi!

In my case, the host is a Raspberry Pi OS host. Once you know how, it's easiest to set up network routing and DHCP relay agents to deal with this.

### Dynamic routing

Somehow, there must be routes to the subnet for the black pill interface. Static routes are
a possibility, but that often involves a lot of repitition.

babeld is fairly simple:

```shell
sudo apt install isc-dhcp-relay
sudo systemctl enable babeld
```

Then `/etc/babeld.conf` for the host:

```conf
# For more information about this configuration file, refer to
# babeld(8)
redistribute ip 192.168.2.0/24 allow
interface enxce567d849b70
interface wlan0
```

On other hosts (like your main router) you don't need the `redistribute`: just the list of interfaces.

Where `192.168.2.0/24` is the network assigned to the black pill, `enxce567d849b70` is
network interface [derived from it's MAC address](#predictable-network-interface-names), and wlan0 is the wifi network we couldn't bridge to.

```shell
sudo /etc/init.d/babeld start
```

### DHCP Relay

Now we just have to arrange for the black pills DHCP request to be responded to.

```shell
sudo apt install isc-dhcp-relay
```

Don't list an interfaces, instead use the options to specify upstream interfaces with `-iu` - the interface or interfaces that have a DHCP server, and downstream interfices with `-id` - the
interfaces you want to provide DHCP relay to - I.e. the black pill interface.

### Predictable network interface names

The code uses the device ID to generate a MAC. If you enable [Predictable Network Interface Names](https://systemd.io/PREDICTABLE_INTERFACE_NAMES/)
each black pill will have a predictable network interface name: `en` followed by the mac address.

