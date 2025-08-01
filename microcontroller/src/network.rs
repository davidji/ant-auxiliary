use rtic_monotonics::Monotonic;
use sha2::{Digest, Sha256};
use core::{future::poll_fn, marker::PhantomData};

use futures::task::Poll;

use defmt::{ debug, error, info, warn };

use hal::{
    otg_fs::{ UsbBus, USB }
};

use usbd_ethernet::{ Ethernet, DeviceState };
use usb_device::UsbError;

use smoltcp::{
    iface::{self, Interface, SocketHandle, SocketSet, SocketStorage }, 
    socket::{ dhcpv4, tcp },  
    time::Instant, 
    wire::{ DhcpOption, EthernetAddress, IpCidr, Ipv4Address, Ipv4Cidr }
};

use rtic_sync::channel::{ Channel, ReceiveError, Receiver, Sender, TrySendError};

pub const IP_ADDRESS: Ipv4Address = Ipv4Address::new(0, 0, 0, 0);
pub const MTU: u16 = 64;

pub fn mac_address(seed: &str) -> [u8; 6] {
    let uid = hal::signature::Uid::get();
    let mut digest = Sha256::new();
    digest.update(seed.as_bytes());
    digest.update(uid.lot_num());
    digest.update(&uid.waf_num().to_le_bytes());
    digest.update(&uid.x().to_be_bytes());
    digest.update(&uid.y().to_le_bytes());
    let hash = digest.finalize();
    let mut mac = [0u8; 6];
    mac.copy_from_slice(&hash[0..6]);
    // Set the second-least-significant bit to indicate a locally administered address
    mac[0] |= 0b00000010; 
    mac
}

pub fn usb_ethernet<'a>(
    usb_alloc: &'a usb_device::bus::UsbBusAllocator<UsbBus<USB>>,
    in_buffer: &'a mut [u8; 2048],
    out_buffer: &'a mut [u8; 2048]) ->  Ethernet<'a, UsbBus<USB>> {

    let mac_address = mac_address("interface");
    info!("interface MAC address: {}", EthernetAddress(mac_address));
    Ethernet::new(
        usb_alloc,
        mac_address,
        MTU,
        in_buffer,
        out_buffer)
}


#[derive(Clone, Copy)]
enum RecvChannelState {
    Listening,
    Receiving,
    Closing,
}

pub struct RecvChannel<'a, const N: usize> {
    port: u16,
    handle: SocketHandle,
    sender: Sender<'a, u8, N>,
    state: RecvChannelState,
}

impl <const N: usize> RecvChannel<'_, N> {
    pub fn try_recv(&mut self,  sockets: &mut SocketSet<'_>) -> bool {
        let socket: &mut tcp::Socket = sockets.get_mut(self.handle);
        let mut consumed: usize = 0;

        if self.may_recv(socket) {
            let mut buf = [0u8; N];
            // peek at the bytes, because we don't know how many we can forward
            match socket.peek_slice(&mut buf[..]) {
                Ok(received) => {
                    for index in 0..received {
                        match self.sender.try_send(buf[index]) {
                            Ok(()) => { consumed += 1; },
                            Err(TrySendError::Full(_)) => break,
                            Err(TrySendError::NoReceiver(_)) => { panic!("no receiver"); },
                        }
                    }

                    // Read however many bytes we could send to the channel
                    socket.recv_slice(&mut buf[0..consumed]).unwrap();
                    if consumed < received {
                        warn!("sender is full. received {}, consumed {} for {}", received, consumed, self.port);
                    } else {
                        debug!("consumed {} bytes on {}", consumed, self.port);
                    }
                },
                Err(e) => { panic!("Error peeking socket input: {}", e); },
            }
        }

        consumed > 0
    }

    fn may_recv(&mut self, socket: &mut tcp::Socket<'_>) -> bool {
        // If the remote closes the socket, we close the socket too, and return to the 
        // listenning state. It may not be necessary to track the state of the channel
        // separately, but it's simpler (the socket state is complicated), and it makes
        // logging the transitions possible.
        let (state, may_recv) = match (self.state, socket.may_recv()) {
            (RecvChannelState::Listening, true) => {
                info!("accepted connection, state: {} on {}", socket.state(), self.port);
                (RecvChannelState::Receiving, true)
            },
            (RecvChannelState::Receiving, false) => {
                info!("remote closed socket, state: {}, closing", socket.state());
                socket.close();
                (RecvChannelState::Closing, false)
            },
            (RecvChannelState::Closing, false) => {
                match socket.is_active() {
                    true => (RecvChannelState::Closing, false),
                    false => {
                        info!("socket closed, state {}, listenning on {}", socket.state(), self.port);
                        socket.listen(self.port).ok();
                        (RecvChannelState::Listening, false)
                    }
                }
            }
            (state, receive) => (state, receive)
        };
        
        self.state = state;
        may_recv
    }
}
pub struct SendChannel<'a, const N: usize> {
    handle: SocketHandle,
    receiver: Receiver<'a, u8, N>
}

impl <const N: usize> SendChannel<'_, N> {
    pub async fn send(&mut self,  sockets: &mut SocketSet<'_>) -> Result<bool, ReceiveError> {
        poll_fn(|_cx| {

            match self.try_send(sockets) {
                Ok(false) => Poll::Pending,
                Ok(true) => Poll::Ready(Ok(true)),
                Err(err) => return Poll::Ready(Err(err)),
            }
        }).await
    }

    pub fn try_send(&mut self, sockets: &mut SocketSet<'_>) -> Result<bool, ReceiveError> {
        let socket:&mut tcp::Socket = sockets.get_mut(self.handle);

        if socket.may_send() {
            let mut count: usize = 0;
            while socket.can_send() {
                match self.receiver.try_recv() {
                    Ok(data) => {
                        socket.send_slice(&[data]).ok();
                        count += 1;
                    },
                    Err(ReceiveError::Empty) => { 
                        break; 
                    },
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
            Ok(count != 0)
        } else {
            loop {
                match self.receiver.try_recv() {
                    Ok(_) => { },
                    Err(ReceiveError::Empty) => { 
                        return Ok(false);
                    },
                    Err(err) => {
                        return Err(err);
                    }
                }
            }
        }
    }   
}

pub struct NetworkChannelStorage<const N: usize> {
    pub sender: Channel<u8, N>,
    pub receiver: Channel<u8, N>,
    pub tx_storage: [u8; N],
    pub rx_storage: [u8; N],
}

impl  <const N: usize> NetworkChannelStorage<N> {

    pub const fn new() -> Self {
        Self {
            sender: Channel::new(),
            receiver: Channel::new(),
            tx_storage: [0x0; N],
            rx_storage: [0x0; N],
        }
    }
}

pub struct NetworkEndpoint<'a, const N: usize> {
    pub send: SendChannel<'a, N>,
    pub recv: RecvChannel<'a, N>,
}

pub struct ApplicationEndpoint<'a, const N: usize> {
    pub send: Sender<'a, u8, N>,
    pub recv: Receiver<'a, u8, N>,
}

pub struct NetworkChannel<'a, const N: usize> {
    pub net: NetworkEndpoint<'a, N>,
    pub app: ApplicationEndpoint<'a, N>
}

pub trait IntoInstant {
    fn into_instant(self) -> Instant;    
}

pub struct NetworkStack<'a, CLOCK: Monotonic> {
    pub ethernet: Ethernet<'a, UsbBus<USB>>,
    pub interface: Interface,
    pub sockets: SocketSet<'a>,
    pub dhcp: SocketHandle,
    clock: PhantomData<CLOCK>,
}

impl <'a, CLOCK: Monotonic> NetworkStack<'a, CLOCK>
where CLOCK::Instant: IntoInstant {

    pub fn new(
        options:&'static [DhcpOption<'static>],
        mut ethernet: Ethernet<'a, UsbBus<USB>>,
        storage: &'a mut [SocketStorage<'a>],
        seed: u64) -> Self {
        let interface = Self::interface(&mut ethernet, seed);
        let mut dhcp_socket = dhcpv4::Socket::new();
        dhcp_socket.set_outgoing_options(options);
        let mut sockets = SocketSet::new(storage);
        let dhcp = sockets.add(dhcp_socket);
        NetworkStack::<'a,CLOCK> {
            ethernet,
            interface,
            sockets,
            dhcp,
            clock: PhantomData
        }
    }

    fn interface(ethernet: &mut Ethernet<'a, UsbBus<USB>>, seed: u64) -> Interface {
        let mac_address = EthernetAddress(mac_address("device"));
        let mut interface_config = iface::Config::new(mac_address.into());
        interface_config.random_seed = seed;

        let mut interface = Interface::new(
            interface_config,
            ethernet,
            Self::now());

        interface.update_ip_addrs(|ip_addrs| {
            ip_addrs
                .push(Ipv4Cidr::new(IP_ADDRESS, 0).into())
                .unwrap();
        });

        info!("device MAC address: {}", mac_address);
        interface
    }

    pub fn connect(&mut self)  {
        if self.ethernet.state() == DeviceState::Disconnected {
            if self.ethernet.connection_speed().is_none() {
                // 1000 Kps upload and download
                match self.ethernet.set_connection_speed(1_000_000, 1_000_000) {
                    Ok(()) | Err(UsbError::WouldBlock) => {}
                    Err(e) => error!("Failed to set connection speed: {}", e),
                }
            } else if self.ethernet.state() == DeviceState::Disconnected {
                match self.ethernet.connect() {
                    Ok(()) | Err(UsbError::WouldBlock) => {}
                    Err(e) => error!("Failed to connect: {}", e),
                }
            }
        }
    }

    pub fn connected(&mut self) -> bool {
        self.ethernet.state() == DeviceState::Connected
    }
    
    pub fn try_send<const N: usize>(&mut self, channels: &mut [SendChannel<N>]) {
        if self.connected() {
            let mut data = false;
            for channel in channels {
                data |= match channel.try_send(&mut self.sockets) {
                    Ok(sent) => sent,
                    Err(_) => panic!("Error reading from channel reciever")
                }
            }

            if data {
                self.interface.poll_egress(Self::now(), &mut self.ethernet, &mut self.sockets);
            }
        } else {
            self.connect();
        }
    }

    pub fn try_recv<const N: usize>(&mut self, channels: &mut [RecvChannel<N>]) {
        let data = match self.interface.poll(Self::now(), &mut self.ethernet, &mut self.sockets) {
            iface::PollResult::SocketStateChanged => true,
            iface::PollResult::None => false
        };

        self.dhcp_poll();

        let mut ack = false;
        if data {
            for channel in channels {
                ack |= channel.try_recv(&mut self.sockets);
            }
        }

        if ack {
            self.interface.poll_egress(Self::now(), &mut self.ethernet, &mut self.sockets);
        }
    }

    fn dhcp_poll(&mut self) {
        let event = self.sockets.get_mut::<dhcpv4::Socket>(self.dhcp).poll();
        match event {
            None => {}
            Some(dhcpv4::Event::Configured(config)) => {
                debug!("DHCP config acquired!");

                info!("IP address:      {}", config.address);
                self.interface.update_ip_addrs(|addrs| {
                    addrs.clear();
                    addrs.push(IpCidr::Ipv4(config.address)).unwrap();
                });

                if let Some(router) = config.router {
                    debug!("Default gateway: {}", router);
                    self.interface.routes_mut().add_default_ipv4_route(router).unwrap();
                } else {
                    debug!("Default gateway: None");
                    self.interface.routes_mut().remove_default_ipv4_route();
                }

                for (i, s) in config.dns_servers.iter().enumerate() {
                    debug!("DNS server {}:    {}", i, s);
                }
            }
            Some(dhcpv4::Event::Deconfigured) => {
                debug!("DHCP lost config!");
                self.interface.update_ip_addrs(|addrs| addrs.clear());
                self.interface.routes_mut().remove_default_ipv4_route();
            }
        }

    }

    pub fn channel<const N:usize>(&mut self, port: u16, storage: &'a mut NetworkChannelStorage<N>) -> NetworkChannel<'a, N> {
        let rx_buffer = tcp::SocketBuffer::new(&mut storage.rx_storage[..]);
        let tx_buffer = tcp::SocketBuffer::new(&mut storage.tx_storage[..]);

        let socket = tcp::Socket::new(rx_buffer, tx_buffer);
        let handle = self.sockets.add(socket);
    
        let socket = self.sockets.get_mut::<tcp::Socket>(handle);
        socket.listen(port).ok();
      
        let (net_send, app_recv) = storage.receiver.split();
        let (app_send, net_recv) = storage.sender.split();

        NetworkChannel {
            net: NetworkEndpoint { 
                send: SendChannel { handle, receiver: net_recv },
                recv: RecvChannel { port, handle, sender: net_send, state: RecvChannelState::Listening },
            },
            app: ApplicationEndpoint { send: app_send, recv: app_recv }
        }
    }

    fn now() -> Instant {
        CLOCK::now().into_instant()
    }
}
